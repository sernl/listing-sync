//! The wire vocabulary survives a round trip.
//!
//! These are the shapes the desktop client will import, and `tam-api` puts the
//! same types on the wire rather than restating them. A round trip is the
//! cheapest proof that the definition is genuinely one definition: a field that
//! serialises but cannot be read back is a client that parses the wrong shape,
//! and it would otherwise not be noticed until a device existed.

use tam_domain::{ItemOperation, ItemOutcome, JobItemId, StepBudget};
use tam_engine_driver::driver::VerifyPolicy;
use tam_engine_driver::vocabulary::{
    AttemptIntent, AttemptRef, AttemptVerdict, BindDisposition, BudgetGrant, ClaimView, GrantKind,
    ItemPreparation, ItemVerdict, LandingEffect, LeaseRef, LeasedItem, LedgerAnswer, LedgerCall,
    LedgerError, NewAttempt, PayloadManifest, PreflightStreak, ReconcileSubject, SettleEnvelope,
    WorkOrder,
};
use tam_marketplace::{
    CreateStrategy, FormId, IdempotencyKey, LifecycleTransition, ListingState, RemoteLifecycle,
    RemoteListingId,
};
use tam_types::{FailureCode, FailureDetail, InventoryId, JobId, MappingId, OrgId, Uuid};

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
        inventory: InventoryId::TesGb,
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
        inventory: InventoryId::TesGb,
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
            hash: tam_types::ContentHash([0x0A; 32]),
            byte_len: 4_096,
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
            inventory: InventoryId::TesGb,
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
                landing: LandingEffect::None,
            },
            at_ms: 1_756_000_002_000,
        },
        LedgerCall::GateConnection {
            lease: lease(),
            inventory: InventoryId::TesGb,
            at_ms: 1_756_000_003_000,
        },
        LedgerCall::HaltThisTenant {
            lease: lease(),
            inventory: InventoryId::TesGb,
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
