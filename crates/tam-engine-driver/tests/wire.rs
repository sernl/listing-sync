//! The wire vocabulary survives a round trip.
//!
//! These are the shapes the desktop client will import, and `tam-api` puts the
//! same types on the wire rather than restating them. A round trip is the
//! cheapest proof that the definition is genuinely one definition: a field that
//! serialises but cannot be read back is a client that parses the wrong shape,
//! and it would otherwise not be noticed until a device existed.

use tam_domain::{ItemOperation, ItemOutcome, JobItemId};
use tam_engine_driver::driver::VerifyPolicy;
use tam_engine_driver::vocabulary::{
    AttemptIntent, AttemptRef, AttemptVerdict, BindDisposition, BudgetGrant, ClaimView, GrantKind,
    ItemPreparation, ItemVerdict, LandingEffect, LeaseRef, LeasedItem, LedgerError, NewAttempt,
    PayloadManifest, PreflightStreak, SettleEnvelope, WorkOrder,
};
use tam_marketplace::{
    IdempotencyKey, LifecycleTransition, ListingState, RemoteLifecycle, RemoteListingId,
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
    };
    assert_eq!(
        round_trip(&envelope),
        envelope,
        "the fence the device carries back must be the fence it was issued"
    );
}
