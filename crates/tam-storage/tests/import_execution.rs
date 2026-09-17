//! Who owns an import, and what an owner that has lost it may still do.
//!
//! Every case here is one of the failures the execution protocol exists to
//! close, and each is asserted against stored rows rather than against a
//! helper's return value alone.
//!
//! The fence: a device that has been taken over cannot renew, cannot report
//! progress and cannot be told it still holds the run. The terminal rule: a
//! settled run is not reopened by a late report, and accepted counts do not
//! regress. The start key: a lost acknowledgement replayed after settlement
//! reaches the run it already made, and the same key spent on a different
//! shop — or on a retry of a different run — is refused rather than answered
//! with the wrong import. The activation window: a manual run nobody claimed
//! becomes an actionable failure, and a scheduled one waiting for a device
//! does not. And the fence that replaced the organisation-wide one: two shops
//! advance together, and one shop twice does not. The narration: a reported
//! refusal outlives the reclaim that resumes from it, and is retired by
//! accepted work rather than by another attempt.

#![cfg(feature = "pg-tests")]

use sqlx::{Executor, PgPool};
use tam_storage::{
    ClaimOutcome, DeviceRegistration, DeviceRepo, FenceOutcome, ImportRunRepo, ImportStage,
    NewImportRun, ProgressReport, RunKind, RunOpening, RunState,
};
use tam_types::{InventoryId, JobId, OrgId, Timestamp, Uuid};

mod common;
use common::{minimal_product, seed_org_a, ORG_A, PRODUCT_1};

const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const RUN_A: Uuid = Uuid([0x0A; 16]);
const RUN_B: Uuid = Uuid([0x0B; 16]);
const KEY_1: Uuid = Uuid([0x11; 16]);
const NOW: Timestamp = Timestamp(1_000_000);
const PHONE: &str = "1111222233334444aaaabbbbccccdddd";
const TABLET: &str = "5555666677778888eeeeffff00001111";

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_devices(pool: &PgPool, org: OrgId) {
    let devices = DeviceRepo::new(pool.clone());
    for id in [PHONE, TABLET] {
        devices
            .register(
                org,
                &DeviceRegistration {
                    id,
                    name: "a test machine",
                    os: "android",
                    arch: "aarch64",
                    app_version: "0.2.0",
                },
                NOW,
            )
            .await
            .expect("the device registers");
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn anchor(pool: &PgPool, org: OrgId, run: Uuid, source: InventoryId) -> JobId {
    let minted = tam_storage::JobRepo::new(pool.clone())
        .create_with_request_key(
            org,
            tam_storage::JobOrigin {
                request_key: tam_storage::job_request_key(run, tam_storage::IMPORT_LEG),
                run: None,
                import_run: None,
            },
            &tam_storage::NewJob {
                job: JobId(Uuid(*uuid::Uuid::new_v4().as_bytes())),
                inventory: source,
                stamp: tam_types::Stamp::system(tam_types::SystemComponent::Import, NOW),
            },
            &[],
        )
        .await
        .expect("the anchor job mints");
    match minted {
        tam_storage::Minted::Job(created) => Some(created.job),
        tam_storage::Minted::WorkflowDeleted(_) => None,
    }
    .expect("an anchor job names no workflow that could refuse it")
}

fn new_run(id: Uuid, source: InventoryId, anchor_job: JobId) -> NewImportRun {
    NewImportRun {
        id,
        kind: RunKind::Marketplace,
        source: Some(source),
        batch_id: None,
        target: None,
        anchor_job,
        created_at: NOW,
        scheduled: false,
        start_key: None,
        retry_of: None,
    }
}

/// A taken-over device writes nothing more: not a renewal, not a progress
/// report, and not a fence that claims it still holds the run.
#[sqlx::test(migrations = "./migrations")]
async fn takeover_silences_the_old_device(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    seed_devices(&pool, ORG_A).await;
    let runs = ImportRunRepo::new(pool.clone());
    let job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    assert_eq!(
        runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, job))
            .await
            .expect("the run opens"),
        RunOpening::Opened,
    );

    let ClaimOutcome::Granted(first) = runs
        .claim(ORG_A, RUN_A, PHONE, false)
        .await
        .expect("the claim answers")
    else {
        panic!("an unclaimed run is claimable");
    };

    // Another device cannot take it without saying so.
    let refused = runs
        .claim(ORG_A, RUN_A, TABLET, false)
        .await
        .expect("the claim answers");
    assert!(
        matches!(refused, ClaimOutcome::HeldBy { ref device, .. } if device == PHONE),
        "a held run names its owner rather than changing hands silently: {refused:?}"
    );

    let ClaimOutcome::Granted(second) = runs
        .claim(ORG_A, RUN_A, TABLET, true)
        .await
        .expect("the takeover answers")
    else {
        panic!("a takeover is granted");
    };
    assert!(
        second.attempt > first.attempt,
        "a takeover raises the fence"
    );

    assert!(
        runs.renew(ORG_A, RUN_A, PHONE, first.attempt)
            .await
            .expect("the renewal answers")
            .is_none(),
        "the old device cannot renew after a takeover"
    );
    assert!(
        runs.report_progress(
            ORG_A,
            RUN_A,
            PHONE,
            &ProgressReport {
                attempt: first.attempt,
                stage: ImportStage::Reading,
                discovered: 40,
                processed: 5,
                reason_code: None,
                reason: None,
            },
        )
        .await
        .expect("the report answers")
        .is_none(),
        "the old device cannot report progress after a takeover"
    );
    let head = runs
        .head(ORG_A, RUN_A)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        head.execution.discovered, 0,
        "a refused report writes no counts"
    );
    assert_eq!(head.execution.owner_device.as_deref(), Some(TABLET));
    assert_eq!(
        runs.fence(ORG_A, RUN_A, PHONE, first.attempt)
            .await
            .expect("the fence answers"),
        Some(FenceOutcome::NotOwner {
            owner: Some(TABLET.to_owned())
        }),
    );
    Ok(())
}

/// A late report cannot reopen a run that stopped, and cannot walk its counts
/// backwards while it is open.
#[sqlx::test(migrations = "./migrations")]
async fn terminal_runs_and_accepted_counts_stand(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    seed_devices(&pool, ORG_A).await;
    let runs = ImportRunRepo::new(pool.clone());
    let job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, job))
        .await
        .expect("the run opens");
    let ClaimOutcome::Granted(lease) = runs
        .claim(ORG_A, RUN_A, PHONE, false)
        .await
        .expect("the claim answers")
    else {
        panic!("an unclaimed run is claimable");
    };

    runs.report_progress(
        ORG_A,
        RUN_A,
        PHONE,
        &ProgressReport {
            attempt: lease.attempt,
            stage: ImportStage::Discovering,
            discovered: 120,
            processed: 3,
            reason_code: None,
            reason: None,
        },
    )
    .await
    .expect("the report answers")
    .expect("the owner's report is accepted");

    // A device that restarted its own counter is not a shop that shrank.
    runs.report_progress(
        ORG_A,
        RUN_A,
        PHONE,
        &ProgressReport {
            attempt: lease.attempt,
            stage: ImportStage::Discovering,
            discovered: 2,
            processed: 0,
            reason_code: None,
            reason: None,
        },
    )
    .await
    .expect("the report answers")
    .expect("the owner's report is accepted");
    let head = runs
        .head(ORG_A, RUN_A)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(head.execution.discovered, 120, "counts never regress");
    assert_eq!(head.execution.processed, 3);

    // The seller stops it, and a later failure report changes nothing.
    runs.set_state(
        ORG_A,
        RUN_A,
        RunState::Abandoned,
        Some("you stopped this import"),
        NOW,
    )
    .await
    .expect("the run settles");
    assert!(
        runs.report_progress(
            ORG_A,
            RUN_A,
            PHONE,
            &ProgressReport {
                attempt: lease.attempt,
                stage: ImportStage::Failed,
                discovered: 120,
                processed: 3,
                reason_code: Some(tam_storage::ImportReasonCode::EnumerationFailed),
                reason: Some("the shop would not answer"),
            },
        )
        .await
        .expect("the report answers")
        .is_none(),
        "a settled run does not accept a late report"
    );
    let settled = runs
        .head(ORG_A, RUN_A)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        settled.state,
        RunState::Abandoned,
        "a cancelled run is not reopened as failed by a late page"
    );
    assert_eq!(
        settled.failure_detail.as_deref(),
        Some("you stopped this import"),
        "the reason the seller was given is not overwritten"
    );
    Ok(())
}

/// A lost start acknowledgement, replayed: after settlement, and with the key
/// reused for a different shop and for a different retry.
#[sqlx::test(migrations = "./migrations")]
async fn a_spent_start_key_mints_no_second_run(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    let runs = ImportRunRepo::new(pool.clone());
    let job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    let mut first = new_run(RUN_A, InventoryId::Tes, job);
    first.start_key = Some(KEY_1);
    assert_eq!(
        runs.create(ORG_A, &first).await.expect("the run opens"),
        RunOpening::Opened
    );

    // The run completes, and the client's retry still arrives.
    runs.set_state(ORG_A, RUN_A, RunState::Complete, None, NOW)
        .await
        .expect("the run settles");
    let second_job = anchor(&pool, ORG_A, RUN_B, InventoryId::Tes).await;
    let mut replay = new_run(RUN_B, InventoryId::Tes, second_job);
    replay.start_key = Some(KEY_1);
    assert_eq!(
        runs.create(ORG_A, &replay)
            .await
            .expect("the replay answers"),
        RunOpening::AlreadyOpen(RUN_A),
        "a replayed key reaches the run it already made, even after completion"
    );
    assert!(
        runs.head(ORG_A, RUN_B)
            .await
            .expect("the head reads")
            .is_none(),
        "a replay mints no second run"
    );

    // The same key, a different shop: a different intent wearing an answered
    // key.
    let third_job = anchor(&pool, ORG_A, RUN_B, InventoryId::Tpt).await;
    let mut elsewhere = new_run(RUN_B, InventoryId::Tpt, third_job);
    elsewhere.start_key = Some(KEY_1);
    assert_eq!(
        runs.create(ORG_A, &elsewhere)
            .await
            .expect("the conflict answers"),
        RunOpening::KeySpent {
            source: InventoryId::Tes
        },
    );

    // The same key and the same shop, but now claiming to retry the settled
    // run: also a different intent.
    let fourth_job = anchor(&pool, ORG_A, RUN_B, InventoryId::Tes).await;
    let mut retry = new_run(RUN_B, InventoryId::Tes, fourth_job);
    retry.start_key = Some(KEY_1);
    retry.retry_of = Some(RUN_A);
    assert_eq!(
        runs.create(ORG_A, &retry)
            .await
            .expect("the conflict answers"),
        RunOpening::KeySpent {
            source: InventoryId::Tes
        },
    );
    assert!(
        runs.head(ORG_A, RUN_B)
            .await
            .expect("the head reads")
            .is_none(),
        "a refused reuse leaves no run behind"
    );
    Ok(())
}

/// A manual start nobody picked up becomes an actionable failure; a scheduled
/// run that is truthfully waiting for a device does not.
#[sqlx::test(migrations = "./migrations")]
async fn activation_expires_only_for_a_seller_s_own_start(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    let runs = ImportRunRepo::new(pool.clone());
    let manual_job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    let mut manual = new_run(RUN_A, InventoryId::Tes, manual_job);
    // Opened well before the activation window, by the database's own clock:
    // the window is measured against `now()` inside the statement.
    manual.created_at = Timestamp(0);
    runs.create(ORG_A, &manual).await.expect("the run opens");
    let scheduled_job = anchor(&pool, ORG_A, RUN_B, InventoryId::Tpt).await;
    let mut scheduled = new_run(RUN_B, InventoryId::Tpt, scheduled_job);
    scheduled.created_at = Timestamp(0);
    scheduled.scheduled = true;
    runs.create(ORG_A, &scheduled)
        .await
        .expect("the scheduled run opens");

    let expired = runs
        .expire_activations(ORG_A, "no device picked this import up")
        .await
        .expect("the sweep runs");
    assert_eq!(expired, vec![RUN_A], "only the seller's own start expires");
    let failed = runs
        .head(ORG_A, RUN_A)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(failed.state, RunState::Failed);
    assert_eq!(
        failed.execution.reason_code,
        Some(tam_storage::ImportReasonCode::ActivationExpired),
        "the console renders a next action from the code"
    );
    let waiting = runs
        .head(ORG_A, RUN_B)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        waiting.state,
        RunState::Reading,
        "a scheduled run truthfully awaiting a device is left alone"
    );
    Ok(())
}

/// Two shops advance together; one shop twice does not.
#[sqlx::test(migrations = "./migrations")]
async fn the_fence_is_per_source(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    let runs = ImportRunRepo::new(pool.clone());
    let tes_job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, tes_job))
        .await
        .expect("the Tes run opens");
    let tpt_job = anchor(&pool, ORG_A, RUN_B, InventoryId::Tpt).await;
    assert_eq!(
        runs.create(ORG_A, &new_run(RUN_B, InventoryId::Tpt, tpt_job))
            .await
            .expect("the TPT run opens"),
        RunOpening::Opened,
        "another source remains startable while the first is open"
    );
    assert_eq!(
        runs.open_runs(ORG_A)
            .await
            .expect("the open runs read")
            .len(),
        2
    );

    let again = Uuid([0x0C; 16]);
    let again_job = anchor(&pool, ORG_A, again, InventoryId::Tes).await;
    assert_eq!(
        runs.create(ORG_A, &new_run(again, InventoryId::Tes, again_job))
            .await
            .expect("the second Tes start answers"),
        RunOpening::AlreadyOpen(RUN_A),
        "a second start on one shop links to the import already open on it"
    );
    Ok(())
}

/// Another tenant's run is not claimable, and the fence says nothing about it.
#[sqlx::test(migrations = "./migrations")]
async fn a_run_is_not_reachable_across_tenants(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-b', now())")
        .bind(uuid::Uuid::from_bytes(ORG_B.0 .0))
        .execute(&pool)
        .await?;
    seed_devices(&pool, ORG_B).await;
    let runs = ImportRunRepo::new(pool.clone());
    let job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, job))
        .await
        .expect("the run opens");

    assert!(
        runs.head(ORG_B, RUN_A)
            .await
            .expect("the head reads")
            .is_none(),
        "another tenant's run does not read back"
    );
    assert!(
        runs.fence(ORG_B, RUN_A, PHONE, 1)
            .await
            .expect("the fence answers")
            .is_none(),
        "another tenant's run has no fence to fail"
    );
    Ok(())
}

/// A hold that lapsed refuses as surely as a stale fence, and a late failure
/// under it cannot settle the run.
///
/// The window this closes is the one a suspended phone opens: it wakes an
/// hour later still holding the attempt it was granted, and before this both
/// the page fence and the progress write accepted it — the second even
/// renewing the lease it had already lost, and a `failed` stage settling a
/// run the console had shown as interrupted.
#[sqlx::test(migrations = "./migrations")]
async fn a_lapsed_hold_writes_nothing(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    seed_devices(&pool, ORG_A).await;
    let runs = ImportRunRepo::new(pool.clone());
    let job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, job))
        .await
        .expect("the run opens");
    let ClaimOutcome::Granted(lease) = runs
        .claim(ORG_A, RUN_A, PHONE, false)
        .await
        .expect("the claim answers")
    else {
        panic!("an unclaimed run is claimable");
    };

    // The lease lapses. Expressed as the maintenance pass sees it — by the
    // database's own clock — rather than by waiting a minute in a test.
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE import_run SET lease_expires_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(uuid::Uuid::from_bytes(RUN_A.0))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    assert_eq!(
        runs.fence(ORG_A, RUN_A, PHONE, lease.attempt)
            .await
            .expect("the fence answers"),
        Some(FenceOutcome::Expired),
        "the owner's own attempt is refused once its hold has lapsed"
    );
    assert!(
        runs.report_progress(
            ORG_A,
            RUN_A,
            PHONE,
            &ProgressReport {
                attempt: lease.attempt,
                stage: ImportStage::Failed,
                discovered: 9,
                processed: 9,
                reason_code: Some(tam_storage::ImportReasonCode::EnumerationFailed),
                reason: Some("the shop stopped answering"),
            },
        )
        .await
        .expect("the report answers")
        .is_none(),
        "a lapsed hold cannot report, and so cannot settle the run"
    );
    let head = runs
        .head(ORG_A, RUN_A)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(head.state, RunState::Reading, "the run is still open");
    assert_eq!(head.execution.discovered, 0, "and nothing was written");
    assert!(
        !head.execution.lease_live,
        "the hold stays lapsed until the device claims again"
    );

    // Claiming again is the remedy, and it is a new attempt.
    let ClaimOutcome::Granted(again) = runs
        .claim(ORG_A, RUN_A, PHONE, false)
        .await
        .expect("the claim answers")
    else {
        panic!("the owner may reclaim its own run");
    };
    assert!(again.attempt > lease.attempt);
    assert_eq!(
        runs.fence(ORG_A, RUN_A, PHONE, again.attempt)
            .await
            .expect("the fence answers"),
        Some(FenceOutcome::Current),
    );
    Ok(())
}

/// A start answered with the run already open on its shop still spends its
/// key on that run.
///
/// Without the binding the answer was not retained: a lost acknowledgement
/// retried after that run settled found no key, opened a second import, and
/// the same key could then be spent on another shop.
#[sqlx::test(migrations = "./migrations")]
async fn a_linked_start_retains_its_key(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    let runs = ImportRunRepo::new(pool.clone());
    let first = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, first))
        .await
        .expect("the first run opens");

    // A second start on the same shop, under a key of its own: the answer is
    // the run already open, and the key is spent on it.
    let second = anchor(&pool, ORG_A, RUN_B, InventoryId::Tes).await;
    let mut linked = new_run(RUN_B, InventoryId::Tes, second);
    linked.start_key = Some(KEY_1);
    assert_eq!(
        runs.create(ORG_A, &linked)
            .await
            .expect("the start answers"),
        RunOpening::AlreadyOpen(RUN_A),
    );
    assert_eq!(
        runs.start_key_run(ORG_A, KEY_1)
            .await
            .expect("the key reads")
            .map(|binding| binding.run),
        Some(RUN_A),
        "the key the seller was answered under is retained"
    );

    // The run settles, and the client's retry arrives.
    runs.set_state(ORG_A, RUN_A, RunState::Complete, None, NOW)
        .await
        .expect("the run settles");
    let third = anchor(&pool, ORG_A, RUN_B, InventoryId::Tes).await;
    let mut retry = new_run(RUN_B, InventoryId::Tes, third);
    retry.start_key = Some(KEY_1);
    assert_eq!(
        runs.create(ORG_A, &retry)
            .await
            .expect("the replay answers"),
        RunOpening::AlreadyOpen(RUN_A),
        "the retry reaches the run it was answered with rather than minting another"
    );
    assert!(
        runs.head(ORG_A, RUN_B)
            .await
            .expect("the head reads")
            .is_none(),
        "and no second run exists"
    );

    // Nor can that key now be spent on the other shop.
    let fourth = anchor(&pool, ORG_A, RUN_B, InventoryId::Tpt).await;
    let mut elsewhere = new_run(RUN_B, InventoryId::Tpt, fourth);
    elsewhere.start_key = Some(KEY_1);
    assert_eq!(
        runs.create(ORG_A, &elsewhere)
            .await
            .expect("the conflict answers"),
        RunOpening::KeySpent {
            source: InventoryId::Tes
        },
    );
    Ok(())
}

/// A settled run's own record is not rewritten by a later decision.
///
/// Both directions of the race: a stop that read an unlocked head before the
/// last commit completed, and a batch settling a run the seller had already
/// stopped. The first writer's state, instant and sentence stand, and the
/// second is told it did not apply.
#[sqlx::test(migrations = "./migrations")]
async fn a_settled_run_keeps_its_own_ending(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    let runs = ImportRunRepo::new(pool.clone());
    let job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, job))
        .await
        .expect("the run opens");

    assert!(
        runs.set_state(ORG_A, RUN_A, RunState::Complete, None, NOW)
            .await
            .expect("the settle answers"),
        "the first transition applies"
    );
    assert!(
        !runs
            .set_state(
                ORG_A,
                RUN_A,
                RunState::Abandoned,
                Some("you stopped this import"),
                NOW,
            )
            .await
            .expect("the stop answers"),
        "a stop that lost the race does not rewrite a completed import"
    );
    let head = runs
        .head(ORG_A, RUN_A)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(head.state, RunState::Complete);
    assert_eq!(
        head.failure_detail, None,
        "and it is not given a reason it never had"
    );
    Ok(())
}

/// A selection is refused over a half-walked shop, and its frozen total
/// counts what it actually took.
///
/// Two defects in one scenario. Selecting after the first discovery page
/// froze a prefix of the seller's catalogue as the whole of it, leaving the
/// later rows listed and the commit outstanding forever. And the denominator
/// counted a listing discovery had already skipped as held, so a
/// one-resource pass reported itself as one of two.
#[sqlx::test(migrations = "./migrations")]
async fn selection_needs_a_walked_shop_and_freezes_what_it_took(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    let runs = ImportRunRepo::new(pool.clone());
    let job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, job))
        .await
        .expect("the run opens");
    let listed = [
        tam_storage::ListedRow {
            locator: "https://www.tes.com/x/1".to_owned(),
            title: "Already held".to_owned(),
            price: None,
        },
        tam_storage::ListedRow {
            locator: "https://www.tes.com/x/2".to_owned(),
            title: "New this week".to_owned(),
            price: None,
        },
    ];
    runs.append_listed(ORG_A, RUN_A, &listed, NOW)
        .await
        .expect("the list lands");

    // Discovery is not closed yet, so nothing may be ticked from it.
    assert_eq!(
        runs.select(ORG_A, RUN_A, tam_storage::Selection::All, NOW)
            .await
            .expect("the selection answers"),
        tam_storage::SelectionOutcome::Discovering,
    );

    // The shop is walked, and the first listing turns out to be one the
    // catalogue already holds.
    let mut tx = pool.begin().await?;
    tam_storage::pin_tenant(&mut tx, ORG_A)
        .await
        .expect("the tenant pins");
    tam_storage::close_enumeration(&mut tx, ORG_A, RUN_A)
        .await
        .expect("discovery closes");
    tam_storage::record_skipped(
        &mut tx,
        ORG_A,
        tam_storage::ItemAddress {
            run: RUN_A,
            locator: "https://www.tes.com/x/1",
        },
        "already in Resources as Already held",
        NOW,
    )
    .await
    .expect("the held listing settles");
    tx.commit().await?;

    assert_eq!(
        runs.select(ORG_A, RUN_A, tam_storage::Selection::All, NOW)
            .await
            .expect("the selection answers"),
        tam_storage::SelectionOutcome::Taken { selected: 1 },
    );
    let head = runs
        .head(ORG_A, RUN_A)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        head.execution.selected_total,
        Some(1),
        "the denominator is what the seller decided about, not what discovery had settled"
    );

    // A replayed selection is the same decision rather than a smaller one.
    runs.select(ORG_A, RUN_A, tam_storage::Selection::All, NOW)
        .await
        .expect("the replay answers");
    assert_eq!(
        runs.head(ORG_A, RUN_A)
            .await
            .expect("the head reads")
            .and_then(|head| head.execution.selected_total),
        Some(1),
        "and the frozen total does not move under it"
    );

    // A stopped run takes no selection at all.
    runs.set_state(
        ORG_A,
        RUN_A,
        RunState::Abandoned,
        Some("you stopped this import"),
        NOW,
    )
    .await
    .expect("the run settles");
    assert_eq!(
        runs.select(ORG_A, RUN_A, tam_storage::Selection::All, NOW)
            .await
            .expect("the selection answers"),
        tam_storage::SelectionOutcome::Settled(RunState::Abandoned),
    );
    Ok(())
}

/// The device's own stop, decided and written under one lock.
///
/// The failure this closes is a race rather than a rule: a fence read in one
/// transaction followed by a settlement in another lets a takeover land
/// between them, and a phone draining an hour-old queue then abandons the run
/// the new owner is reading. `stop_owned` takes the row `FOR UPDATE` and
/// decides there, so the attempt that stops the run is the attempt that holds
/// it. Driven through the repository because the ordering is the claim.
#[sqlx::test(migrations = "./migrations")]
async fn only_the_holding_attempt_stops_a_run(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    seed_devices(&pool, ORG_A).await;
    let runs = ImportRunRepo::new(pool.clone());
    let job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, job))
        .await
        .expect("the run opens");

    let ClaimOutcome::Granted(first) = runs
        .claim(ORG_A, RUN_A, PHONE, false)
        .await
        .expect("the claim answers")
    else {
        panic!("an unclaimed run is claimable");
    };
    let ClaimOutcome::Granted(second) = runs
        .claim(ORG_A, RUN_A, TABLET, true)
        .await
        .expect("the takeover answers")
    else {
        panic!("a takeover is granted");
    };

    // The old device's queued stop, arriving after the takeover.
    assert_eq!(
        runs.stop_owned(ORG_A, RUN_A, PHONE, first.attempt, NOW)
            .await
            .expect("the stop answers"),
        Some(tam_storage::StopOutcome::Fenced(FenceOutcome::NotOwner {
            owner: Some(TABLET.to_owned())
        })),
        "a stop from a device that no longer holds the run settles nothing"
    );
    assert_eq!(
        runs.head(ORG_A, RUN_A)
            .await
            .expect("the head reads")
            .expect("the run stands")
            .state,
        RunState::Reading,
        "and the run the new owner is reading is still open"
    );

    // The same device under a stale attempt: its own run, not its own fence.
    assert_eq!(
        runs.stop_owned(ORG_A, RUN_A, TABLET, first.attempt, NOW)
            .await
            .expect("the stop answers"),
        Some(tam_storage::StopOutcome::Fenced(FenceOutcome::Stale {
            attempt: second.attempt
        })),
    );

    // The holder's own stop, and then its replay.
    assert_eq!(
        runs.stop_owned(ORG_A, RUN_A, TABLET, second.attempt, NOW)
            .await
            .expect("the stop answers"),
        Some(tam_storage::StopOutcome::Stopped),
    );
    assert_eq!(
        runs.stop_owned(ORG_A, RUN_A, TABLET, second.attempt, NOW)
            .await
            .expect("the stop answers"),
        Some(tam_storage::StopOutcome::Settled(RunState::Abandoned)),
        "a replayed stop reads the abandonment it already made rather than refusing"
    );
    assert!(
        runs.stop_owned(ORG_B, RUN_A, TABLET, second.attempt, NOW)
            .await
            .expect("the stop answers")
            .is_none(),
        "another tenant's run has nothing to stop"
    );
    Ok(())
}

/// What `described` counts, which a taking-over device resumes from.
///
/// Three numbers disagree on purpose here. The shop listed three resources,
/// the device described one and failed on another, and a reported `processed`
/// of two counts both. Only the stored descriptions say how much of the shop
/// is actually done, and a device with no journal that trusted `processed`
/// would skip a resource nobody read.
#[sqlx::test(migrations = "./migrations")]
async fn described_counts_stored_descriptions_rather_than_reports(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    seed_devices(&pool, ORG_A).await;
    let runs = ImportRunRepo::new(pool.clone());
    let job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, job))
        .await
        .expect("the run opens");

    let listed = [
        tam_storage::ListedRow {
            locator: "https://www.tes.com/x/1".to_owned(),
            title: "Fractions pack".to_owned(),
            price: None,
        },
        tam_storage::ListedRow {
            locator: "https://www.tes.com/x/2".to_owned(),
            title: "Long division".to_owned(),
            price: None,
        },
        tam_storage::ListedRow {
            locator: "https://www.tes.com/x/3".to_owned(),
            title: "Shape hunt".to_owned(),
            price: None,
        },
    ];
    runs.append_listed(ORG_A, RUN_A, &listed, NOW)
        .await
        .expect("the list lands");
    assert_eq!(
        runs.described(ORG_A, RUN_A).await.expect("the count reads"),
        0,
        "a listed shop has been enumerated, not described"
    );

    let observed = serde_json::json!({ "locator": "https://www.tes.com/x/1" });
    runs.record_read(
        ORG_A,
        RUN_A,
        &tam_storage::ReadItem {
            locator: "https://www.tes.com/x/1",
            observed: &observed,
            title: "Fractions pack",
            price: None,
            cover_hash: None,
            device: Some(PHONE),
            product: None,
        },
        NOW,
    )
    .await
    .expect("the description lands");
    runs.record_failed(
        ORG_A,
        RUN_A,
        "https://www.tes.com/x/2",
        "the file would not download",
        NOW,
    )
    .await
    .expect("the failure lands");

    assert_eq!(
        runs.described(ORG_A, RUN_A).await.expect("the count reads"),
        1,
        "one description stands: the failure described nothing and the third was never read"
    );
    assert_eq!(
        runs.described(ORG_B, RUN_A).await.expect("the count reads"),
        0,
        "and another tenant reads none of it"
    );
    Ok(())
}

/// A late failure cannot unmake a resource the seller holds.
///
/// The commit records a refusal against an item whose own transaction has
/// already finished, so a stop, a merge or a successful create can land in
/// between. An unconditional write would put `failed` over `imported`, and
/// the report would tell a seller that a product in their catalogue does not
/// exist.
#[sqlx::test(migrations = "./migrations")]
async fn a_late_failure_does_not_unsettle_a_finished_item(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    seed_devices(&pool, ORG_A).await;
    let runs = ImportRunRepo::new(pool.clone());
    let job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, job))
        .await
        .expect("the run opens");

    let locator = "https://www.tes.com/x/1";
    let observed = serde_json::json!({ "locator": locator });
    let product = tam_types::ProductId(Uuid([0xC7; 16]));
    runs.record_read(
        ORG_A,
        RUN_A,
        &tam_storage::ReadItem {
            locator,
            observed: &observed,
            title: "Fractions pack",
            price: None,
            cover_hash: None,
            device: Some(PHONE),
            product: Some(product),
        },
        NOW,
    )
    .await
    .expect("the description lands");
    runs.record_imported(ORG_A, RUN_A, locator, product, NOW)
        .await
        .expect("the item settles");

    let mut tx = pool.begin().await?;
    tam_storage::pin_tenant(&mut tx, ORG_A)
        .await
        .expect("the tenant pins");
    let written = tam_storage::record_failed(
        &mut tx,
        ORG_A,
        RUN_A,
        locator,
        "the file would not download",
        NOW,
    )
    .await
    .expect("the failure answers");
    tx.commit().await?;
    assert!(
        !written,
        "a settled item refuses the late failure rather than taking it"
    );
    assert_eq!(
        runs.counts(ORG_A, RUN_A)
            .await
            .expect("the counts read")
            .imported,
        1,
        "and the resource the seller holds is still imported"
    );

    // The same call on an unsettled item is the ordinary path and still works.
    let mut tx = pool.begin().await?;
    tam_storage::pin_tenant(&mut tx, ORG_A)
        .await
        .expect("the tenant pins");
    let fresh = tam_storage::record_failed(
        &mut tx,
        ORG_A,
        RUN_A,
        "https://www.tes.com/x/2",
        "the file would not download",
        NOW,
    )
    .await
    .expect("the failure answers");
    tx.commit().await?;
    assert!(fresh, "a resource that failed its read records the reason");
    Ok(())
}

/// A selection that owes no work settles the run where it is frozen.
///
/// A seller who ticks nothing has asked for nothing, and no device will ever
/// send a completion for it. Left open, the run sat in `selecting` forever
/// with its shop fenced against the seller's next import, and the device's
/// own list kept offering it work it could not do.
#[sqlx::test(migrations = "./migrations")]
async fn a_selection_that_takes_nothing_settles_the_run(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    seed_devices(&pool, ORG_A).await;
    let runs = ImportRunRepo::new(pool.clone());
    let job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, job))
        .await
        .expect("the run opens");

    let listed = [
        tam_storage::ListedRow {
            locator: "https://www.tes.com/x/1".to_owned(),
            title: "Fractions pack".to_owned(),
            price: None,
        },
        tam_storage::ListedRow {
            locator: "https://www.tes.com/x/2".to_owned(),
            title: "Long division".to_owned(),
            price: None,
        },
    ];
    runs.append_listed(ORG_A, RUN_A, &listed, NOW)
        .await
        .expect("the list lands");
    let mut tx = pool.begin().await?;
    tam_storage::pin_tenant(&mut tx, ORG_A)
        .await
        .expect("the tenant pins");
    tam_storage::close_enumeration(&mut tx, ORG_A, RUN_A)
        .await
        .expect("the shop is walked");
    tx.commit().await?;

    let nothing: [String; 0] = [];
    assert_eq!(
        runs.select(
            ORG_A,
            RUN_A,
            tam_storage::Selection::Locators(&nothing),
            NOW
        )
        .await
        .expect("the selection answers"),
        tam_storage::SelectionOutcome::Taken { selected: 0 },
    );
    let head = runs
        .head(ORG_A, RUN_A)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        head.state,
        RunState::Complete,
        "a run that owes nothing is finished rather than left waiting for a device"
    );
    assert_eq!(
        head.execution.selected_total,
        Some(0),
        "and the frozen total says the choice was made"
    );
    Ok(())
}

/// A parked pair is the seller's question, never the system's claim.
///
/// Migration 0070 states it as a database fact:
/// `duplicate_verdict_system_is_decisive` admits `system` only on a `same`
/// verdict from a decisive layer. The import path parks a pair when a merge
/// it would otherwise have made is withdrawn under the lock, and parking in
/// the scorer's own `system` voice is refused by the check — which inside a
/// page's transaction takes every other write of that page with it. This
/// pins the rule the writers follow.
#[sqlx::test(migrations = "./migrations")]
async fn a_parked_pair_carries_no_system_decision(pool: PgPool) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    let duplicates = tam_storage::DuplicateRepo::new(pool.clone());
    let (lo, hi) = tam_storage::ordered_pair(
        tam_types::ProductId(Uuid([0xE1; 16])),
        tam_types::ProductId(Uuid([0xE2; 16])),
    );
    let evidence = [tam_storage::Evidence {
        layer: tam_storage::MatchLayer::L1,
        polarity: tam_storage::Polarity::Positive,
        measure: 1.0,
        unit: tam_storage::EvidenceUnit::Bytes,
        observed_in: None,
    }];
    let parked = |decided_by| tam_storage::NewVerdict {
        lo,
        hi,
        verdict: tam_storage::Verdict::Parked,
        decided_by,
        winning_layer: tam_storage::MatchLayer::L1,
        log_odds: 6.0,
        fingerprint_version: 1,
        run: None,
        kept: None,
        raised_at: NOW,
        decided_at: None,
        reversible_until: None,
        evidence: &evidence,
    };

    assert!(
        duplicates
            .raise(ORG_A, &parked(tam_storage::DecidedBy::System))
            .await
            .is_err(),
        "the system may not park a pair: it decides only a merge, on a decisive layer"
    );
    duplicates
        .raise(ORG_A, &parked(tam_storage::DecidedBy::Seller))
        .await
        .expect("a park in the seller's voice is the question this table is for");
    Ok(())
}

// The import recovery, migration 0076.
//
// 0074 introduced `selected_total`, `enumeration_complete`, `discovered` and
// `processed`, and ended with an `UPDATE import_run` meant to derive the last
// three for the runs that already existed. That statement moved nothing.
// Migrations are applied as `tam_app`, which owns these tables under FORCE ROW
// LEVEL SECURITY, and a migration runs with no tenant pinned — so the
// organisation-scoped policy hid every row from the backfill, in production as
// well as in dev. `selected_total` was never written at all.
//
// What an old run therefore carries is the column defaults: the enumeration
// reads as open even though the list is complete and `read_total` says so, the
// two counters read as zero even though the run holds described rows, and the
// denominator is absent even where the seller ticked the list days earlier.
// The consequences compound. A closed list with no frozen selection is, to
// every reader, the seller still choosing: `RunPhase::owed` concludes nothing
// is owed, the console draws `selecting`, and the maintenance sweep exempts
// the run — while the run stays open and the per-source fence keeps that shop
// shut against every import the seller starts next.
//
// Seed the legacy rows against schema 0075, run 0076 with no tenant pinned,
// then apply later migrations before asking the current repository to read.
// The pre-upgrade snapshot uses only columns that existed on schema 0075.

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

const SELECTION_BACKFILL: &str = include_str!("../migrations/0076_import_selection_backfill.sql");

/// The version `0076_import_selection_backfill.sql` carries, and therefore the
/// exclusive upper bound of the schema the legacy rows are seeded against.
const SELECTION_BACKFILL_VERSION: i64 = 76;

/// An old scheduled Tes run: the list was walked, the selection was taken, and
/// one selected resource is still waiting to be described.
const RUN_OLD: Uuid = Uuid([0x7A; 16]);
/// An old manual TPT run whose 154 listings nobody has ticked.
const RUN_MANUAL: Uuid = Uuid([0x7B; 16]);
/// An old run that already settled.
const RUN_SETTLED: Uuid = Uuid([0x7C; 16]);
/// A run claimed under a live fence, whose counters are its owner's own
/// reports rather than anything derivable from its rows.
const RUN_FENCED: Uuid = Uuid([0x7D; 16]);

fn as_text(id: Uuid) -> String {
    uuid::Uuid::from_bytes(id.0).to_string()
}

#[expect(
    clippy::panic,
    reason = "a migration that will not apply is a broken fixture, not a failed assertion"
)]
async fn schema_before_selection_backfill(pool: &PgPool) {
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.version < SELECTION_BACKFILL_VERSION)
    {
        pool.execute(&*migration.sql)
            .await
            .unwrap_or_else(|error| panic!("migration {} applies: {error}", migration.version));
    }
}

/// Four runs in the shape a fenced 0074 actually left: `selected_total` NULL,
/// `enumeration_complete` false and both counters zero, whatever the rows and
/// `read_total` say — beside one modern run whose owner reported its own
/// numbers under a live fence.
///
/// The rows are the evidence the recovery has to read. On `RUN_OLD` they are
/// every kind an old run can hold at once: a listing discovery skipped because
/// the catalogue already had it, a listing the seller did not tick, a resource
/// described and then merged away, one described and waiting, and one still
/// owed to a device.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_runs_as_0074_left_them(pool: &PgPool) {
    seed_devices(pool, ORG_A).await;
    let org = as_text(ORG_A.0);
    let old = as_text(RUN_OLD);
    let manual = as_text(RUN_MANUAL);
    let settled = as_text(RUN_SETTLED);
    let fenced = as_text(RUN_FENCED);
    let mut connection = pool.acquire().await.expect("a connection is free");
    connection
        .as_mut()
        .execute(
            format!(
                "SELECT set_config('app.current_org', '{org}', false);

                 INSERT INTO import_run
                     (org_id, id, kind, source, state, anchor_job, read_total, created_at,
                      settled_at, scheduled, enumeration_complete, discovered, processed,
                      owner_device, attempt, lease_expires_at, last_contact_at,
                      last_progress_at, reported_stage)
                 VALUES
                     ('{org}', '{old}', 'marketplace', 'tes', 'reading', gen_random_uuid(), 5,
                      now() - interval '9 days', NULL, true, false, 0, 0,
                      NULL, 0, NULL, NULL, NULL, NULL),
                     ('{org}', '{manual}', 'marketplace', 'tpt', 'reading', gen_random_uuid(),
                      154, now() - interval '8 days', NULL, false, false, 0, 0,
                      NULL, 0, NULL, NULL, NULL, NULL),
                     ('{org}', '{settled}', 'marketplace', 'etsy', 'complete', gen_random_uuid(),
                      1, now() - interval '30 days', now() - interval '29 days', false, false,
                      0, 0, NULL, 0, NULL, NULL, NULL, NULL),
                     ('{org}', '{fenced}', 'marketplace', 'etsy', 'reading', gen_random_uuid(),
                      NULL, now() - interval '2 hours', NULL, false, false, 7, 3,
                      '{PHONE}', 4, now() + interval '5 minutes', now() - interval '1 minute',
                      now() - interval '1 minute', 'discovering');

                 INSERT INTO import_run_item
                     (org_id, run_id, locator, ordinal, state, product_id, observed, title,
                      skip_reason, read_at, settled_at)
                 VALUES
                     ('{org}', '{old}', 'https://www.tes.com/x/1', 1, 'skipped',
                      gen_random_uuid(), NULL, 'Fractions pack',
                      'already in Resources as Fractions pack', now() - interval '9 days',
                      now() - interval '9 days'),
                     ('{org}', '{old}', 'https://www.tes.com/x/2', 2, 'skipped',
                      gen_random_uuid(), NULL, 'Long division', 'not chosen',
                      now() - interval '9 days', now() - interval '9 days'),
                     ('{org}', '{old}', 'https://www.tes.com/x/3', 3, 'skipped',
                      gen_random_uuid(), '{{\"title\": \"Times tables\"}}'::jsonb,
                      'Times tables', 'same as Times tables', now() - interval '9 days',
                      now() - interval '9 days'),
                     ('{org}', '{old}', 'https://www.tes.com/x/4', 4, 'read',
                      gen_random_uuid(), '{{\"title\": \"Number bonds\"}}'::jsonb,
                      'Number bonds', NULL, now() - interval '9 days', NULL),
                     ('{org}', '{old}', 'https://www.tes.com/x/5', 5, 'selected',
                      gen_random_uuid(), NULL, 'Place value', NULL,
                      now() - interval '9 days', NULL),
                     ('{org}', '{settled}', 'https://www.etsy.com/listing/1', 1, 'imported',
                      gen_random_uuid(), '{{\"title\": \"Shapes\"}}'::jsonb, 'Shapes', NULL,
                      now() - interval '30 days', now() - interval '29 days'),
                     ('{org}', '{fenced}', 'https://www.etsy.com/listing/2', 1, 'read',
                      gen_random_uuid(), '{{\"title\": \"Angles\"}}'::jsonb, 'Angles', NULL,
                      now() - interval '1 minute', NULL),
                     ('{org}', '{fenced}', 'https://www.etsy.com/listing/3', 2, 'read',
                      gen_random_uuid(), '{{\"title\": \"Symmetry\"}}'::jsonb, 'Symmetry', NULL,
                      now() - interval '1 minute', NULL);

                 INSERT INTO import_run_item
                     (org_id, run_id, locator, ordinal, state, product_id, title, read_at)
                 SELECT '{org}', '{manual}',
                        'https://www.teacherspayteachers.com/Product/' || n, n, 'listed',
                        gen_random_uuid(), 'Manual listing ' || n, now() - interval '8 days'
                   FROM generate_series(1, 154) AS n;"
            )
            .as_str(),
        )
        .await
        .expect("the legacy runs and their rows seed");
}

/// Run 0076 the way `teachouse-migrate` does: on a connection with no tenant
/// pinned. A migration that only works with one pinned is a migration that
/// does nothing on the host.
async fn apply_selection_backfill(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut connection = pool.acquire().await?;
    connection
        .as_mut()
        .execute("SELECT set_config('app.current_org', '', false);")
        .await?;
    connection.as_mut().execute(SELECTION_BACKFILL).await?;
    Ok(())
}

async fn apply_later_migrations(pool: &PgPool) -> Result<(), sqlx::Error> {
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.version > SELECTION_BACKFILL_VERSION)
    {
        pool.execute(&*migration.sql).await?;
    }
    Ok(())
}

/// An old run whose selection was taken becomes work a device can resume, and
/// keeps every row it already had.
///
/// Two facts are recovered, in that order. First the ones 0074 meant to
/// derive and could not: a `read_total` is the old protocol's own statement
/// that the list closed, and the counters are a count of the run's rows.
/// Then the denominator, which is the selection's recoverable evidence and
/// nothing else: the three rows only a selection could have produced — the
/// resource still owed, the one already described, and the one described and
/// then merged away. The listing the catalogue already held and the listing
/// the seller did not tick were never part of what the seller chose, and
/// counting either would tell the console this run owes work it does not.
#[sqlx::test(migrations = false)]
async fn the_selection_backfill_makes_an_old_taken_selection_resumable(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    schema_before_selection_backfill(&pool).await;
    seed_org_a(&pool).await?;
    seed_runs_as_0074_left_them(&pool).await;
    let runs = ImportRunRepo::new(pool.clone());

    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(as_text(ORG_A.0))
        .execute(&mut *tx)
        .await?;
    let before: (Option<i32>, bool, i32, i32, Option<i32>) = sqlx::query_as(
        "SELECT selected_total, enumeration_complete, discovered, processed, read_total \
         FROM import_run WHERE org_id = $1 AND id = $2",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(RUN_OLD.0))
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    assert_eq!(
        before,
        (None, false, 0, 0, Some(5)),
        "0074 left the execution defaults untouched; only the old protocol's read_total stands"
    );

    apply_selection_backfill(&pool).await?;
    apply_later_migrations(&pool).await?;

    let head = runs
        .head(ORG_A, RUN_OLD)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert!(
        head.execution.enumeration_complete,
        "a `read_total` is the old protocol's own statement that the list closed, so the \
         enumeration is closed again rather than left open against a shop already walked"
    );
    assert_eq!(
        head.execution.selected_total,
        Some(3),
        "the denominator is reconstructed from the rows a selection actually took: the resource \
         still owed, the one described, and the one described and merged away. A listing already \
         in Resources and a listing nobody ticked are not part of it"
    );
    assert_eq!(
        head.state,
        RunState::Reading,
        "the backfill records what was already chosen; it does not move a run between states"
    );
    assert_eq!(
        head.settled_at, None,
        "and it settles nothing that was not settled"
    );
    assert_eq!(
        head.execution.commit_authorised_at, None,
        "an old run authorises no catalogue addition: the confirmation is its own fact, and this \
         migration does not mint one"
    );
    assert_eq!(
        head.execution.owner_device, None,
        "no device is invented for a run nothing recorded an owner for"
    );
    assert_eq!(head.execution.attempt, 0, "and no fence is raised");
    assert_eq!(
        (head.execution.discovered, head.execution.processed),
        (5, 4),
        "and the counters 0074 meant to derive are derived: five rows discovered, and the four \
         that have been read or settled worked through. The selected row still owed is not one \
         of them"
    );

    let counts = runs.counts(ORG_A, RUN_OLD).await.expect("the rows count");
    assert_eq!(
        counts.selected, 1,
        "the resource the seller chose and no device has described is still owed"
    );
    assert_eq!(
        (counts.read, counts.skipped, counts.listed),
        (1, 3, 0),
        "and nothing else about the run's rows moves: a backfill of a run's own column rewrites \
         no item"
    );

    // Applied twice, because a migration that runs once in dev and once in
    // production is the same migration, and a second pass must not re-freeze a
    // denominator against a run that has since moved on.
    apply_selection_backfill(&pool).await?;
    assert_eq!(
        runs.head(ORG_A, RUN_OLD)
            .await
            .expect("the head reads")
            .and_then(|head| head.execution.selected_total),
        Some(3),
        "the backfill keeps the first number it wrote"
    );
    Ok(())
}

/// A list nobody ticked is still the seller's to tick, and a run that already
/// settled stays settled.
///
/// This is the direction the backfill must not overreach in. The 154 listings
/// of the manual TPT run are evidence of a discovery and of nothing else: a
/// denominator here would tell the seller's device to go and describe all of
/// them, which is an import of the whole shop nobody asked for.
#[sqlx::test(migrations = false)]
async fn the_selection_backfill_leaves_an_unticked_list_and_a_settled_run_alone(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    schema_before_selection_backfill(&pool).await;
    seed_org_a(&pool).await?;
    seed_runs_as_0074_left_them(&pool).await;
    let runs = ImportRunRepo::new(pool.clone());

    apply_selection_backfill(&pool).await?;
    apply_later_migrations(&pool).await?;

    let manual = runs
        .head(ORG_A, RUN_MANUAL)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        manual.execution.selected_total, None,
        "a closed list nobody ticked is the seller still choosing, and no evidence of a choice \
         exists to reconstruct"
    );
    assert!(
        manual.execution.enumeration_complete,
        "the list itself is whole, though, and saying so is what makes the console show the \
         seller a choice rather than a shop still being walked"
    );
    assert_eq!(
        (
            manual.execution.discovered,
            manual.execution.processed,
            runs.counts(ORG_A, RUN_MANUAL)
                .await
                .expect("the rows count")
                .listed,
        ),
        (154, 0, 154),
        "all 154 listings were discovered, none has been worked through, and all 154 are still \
         there to be ticked"
    );

    let settled = runs
        .head(ORG_A, RUN_SETTLED)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        settled.state,
        RunState::Complete,
        "a terminal run stays terminal: it is owed no device work, so it needs no denominator \
         and is not reopened to be given one"
    );
    assert_eq!(
        settled.execution.selected_total, None,
        "and its own record is left as it stands"
    );
    assert_eq!(
        (
            settled.execution.enumeration_complete,
            settled.execution.discovered,
            settled.execution.processed,
        ),
        (false, 0, 0),
        "including the counters 0074 never reached: a settled run is a record, not work, and \
         rewriting what it says is not this migration's business"
    );
    assert!(
        settled.settled_at.is_some(),
        "and the instant it settled at stands"
    );
    Ok(())
}

/// A run under a live fence keeps its owner's own numbers.
///
/// This is the other direction the recovery must not overreach in, and the
/// reason it is guarded on `attempt = 0`. Discovery and progress are reported
/// by the device holding the run, and they are not a count of rows: a device
/// that has walked seven listings and posted two of them is telling the truth
/// about a shop the server cannot see. Deriving those counters from the rows
/// would overwrite what the owner said with what has arrived so far, and
/// closing the enumeration behind it would tell the seller to choose from a
/// list still being written.
#[sqlx::test(migrations = false)]
async fn the_selection_backfill_leaves_a_fenced_runs_reports_alone(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    schema_before_selection_backfill(&pool).await;
    seed_org_a(&pool).await?;
    seed_runs_as_0074_left_them(&pool).await;
    let runs = ImportRunRepo::new(pool.clone());

    apply_selection_backfill(&pool).await?;
    apply_later_migrations(&pool).await?;

    let fenced = runs
        .head(ORG_A, RUN_FENCED)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        (fenced.execution.discovered, fenced.execution.processed),
        (7, 3),
        "the owner's reported counts are the owner's, and a backfill does not recount them from \
         the two rows that have landed"
    );
    assert!(
        !fenced.execution.enumeration_complete,
        "and a shop still being walked stays open: only the device closes an enumeration"
    );
    assert_eq!(
        fenced.execution.selected_total, None,
        "nothing is selected on a run whose list is not finished, whatever its described rows \
         would otherwise look like"
    );
    assert_eq!(
        (fenced.execution.attempt, fenced.execution.owner_device),
        (4, Some(PHONE.to_owned())),
        "and the fence and its holder are untouched"
    );
    Ok(())
}

/// The 154-row manual run 0076 recovered, and a manual start nothing was ever
/// recorded against.
///
/// Both are what the activation sweep's `attempt = 0` reads as "no device has
/// ever claimed this", and only one of them is true: the fence is a fact of
/// the protocol 0074 introduced, so no run older than it carries an attempt,
/// however much of the seller's shop it holds. The recovered run is the shape
/// 0076 exists for -- a walked TPT list, a selection taken over it, and half
/// of that selection already described -- and the untouched one is a seller
/// who pressed Import nine days ago on a phone that never answered.
const RUN_RECOVERED: Uuid = Uuid([0x7E; 16]);
const RUN_UNTOUCHED: Uuid = Uuid([0x7F; 16]);

/// Twelve resources still owed to a device and twelve already described, out
/// of a 154-listing shop: the denominator 0076 reconstructs.
const RECOVERED_SELECTION: u32 = 24;

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_a_recovered_run_and_an_untouched_start(pool: &PgPool) {
    seed_devices(pool, ORG_A).await;
    let org = as_text(ORG_A.0);
    let recovered = as_text(RUN_RECOVERED);
    let untouched = as_text(RUN_UNTOUCHED);
    let mut connection = pool.acquire().await.expect("a connection is free");
    connection
        .as_mut()
        .execute(
            format!(
                "SELECT set_config('app.current_org', '{org}', false);

                 INSERT INTO import_run
                     (org_id, id, kind, source, state, anchor_job, read_total, created_at,
                      settled_at, scheduled, enumeration_complete, discovered, processed,
                      owner_device, attempt, lease_expires_at, last_contact_at,
                      last_progress_at, reported_stage)
                 VALUES
                     ('{org}', '{recovered}', 'marketplace', 'tpt', 'reading',
                      gen_random_uuid(), 154, now() - interval '8 days', NULL, false, false,
                      0, 0, NULL, 0, NULL, NULL, NULL, NULL),
                     ('{org}', '{untouched}', 'marketplace', 'tes', 'reading',
                      gen_random_uuid(), NULL, now() - interval '9 days', NULL, false, false,
                      0, 0, NULL, 0, NULL, NULL, NULL, NULL);

                 INSERT INTO import_run_item
                     (org_id, run_id, locator, ordinal, state, product_id, observed, title,
                      read_at)
                 SELECT '{org}', '{recovered}',
                        'https://www.teacherspayteachers.com/Product/' || n, n,
                        CASE WHEN n <= 12 THEN 'selected'
                             WHEN n <= 24 THEN 'read'
                             ELSE 'listed' END,
                        gen_random_uuid(),
                        CASE WHEN n > 12 AND n <= 24
                             THEN ('{{\"title\": \"Manual listing ' || n || '\"}}')::jsonb END,
                        'Manual listing ' || n,
                        now() - interval '8 days'
                   FROM generate_series(1, 154) AS n;"
            )
            .as_str(),
        )
        .await
        .expect("the legacy manual runs and their rows seed");
}

/// The work 0076 recovered survives the maintenance pass that runs next, and
/// an import nothing ever recorded anything against still expires.
///
/// This is the pairing the recovery has to survive. 0076 hands a legacy manual
/// run back to the seller's device, and the very next maintenance pass reaches
/// the same row: it is `reading`, it is not scheduled, it was created days ago
/// and it carries `attempt = 0`, which the activation rule reads as "no device
/// has ever claimed this". Failing it there would settle -- irreversibly, with
/// `activation_expired` against a run nobody was waiting on -- the twelve
/// described resources and the twelve still owed that the migration had just
/// made resumable, so the recovery would last until the sweep's next tick.
///
/// What separates the two runs is not the fence, which neither has and neither
/// could: it is whether anything was ever recorded against the run. The
/// recovered run holds a selection the seller took and descriptions a device
/// wrote; the untouched one holds nothing at all, which is precisely the
/// seller staring at a page while no device answers, and is what the window
/// was approved to turn into a sentence they can act on.
#[sqlx::test(migrations = false)]
async fn maintenance_keeps_the_work_the_selection_backfill_recovered(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    schema_before_selection_backfill(&pool).await;
    seed_org_a(&pool).await?;
    seed_a_recovered_run_and_an_untouched_start(&pool).await;
    let runs = ImportRunRepo::new(pool.clone());

    apply_selection_backfill(&pool).await?;
    apply_later_migrations(&pool).await?;
    let recovered = runs
        .head(ORG_A, RUN_RECOVERED)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        (
            recovered.execution.selected_total,
            recovered.execution.enumeration_complete,
            recovered.execution.attempt,
        ),
        (Some(RECOVERED_SELECTION), true, 0),
        "the fixture starts where 0076 leaves it: the selection is frozen, the walked list is \
         closed, and no fence was raised -- which is the state the sweep then reads"
    );

    let expired = runs
        .expire_activations(ORG_A, "no device picked this import up")
        .await
        .expect("the sweep runs");
    assert_eq!(
        expired,
        vec![RUN_UNTOUCHED],
        "only the run nothing was ever recorded against expires: a seller watching a page no \
         device answered is told so, and a recovered import is not settled underneath them"
    );

    let kept = runs
        .head(ORG_A, RUN_RECOVERED)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        (kept.state, kept.settled_at, kept.execution.reason_code),
        (RunState::Reading, None, None),
        "the recovered run is still open work, unsettled and un-narrated: nothing happened to it \
         that the seller has to be told about"
    );
    assert_eq!(
        kept.execution.selected_total,
        Some(RECOVERED_SELECTION),
        "and it still owes exactly what the seller chose"
    );
    let counts = runs
        .counts(ORG_A, RUN_RECOVERED)
        .await
        .expect("the rows count");
    assert_eq!(
        (counts.selected, counts.read, counts.listed),
        (12, 12, 130),
        "with its rows where they were: twelve resources owed, twelve described, and the rest of \
         the shop untaken"
    );
    assert!(
        matches!(
            runs.claim(ORG_A, RUN_RECOVERED, PHONE, false)
                .await
                .expect("the claim answers"),
            ClaimOutcome::Granted(_)
        ),
        "which is what resumable means here: a device can claim the run and carry on describing \
         the selection, which is the whole point of the recovery"
    );

    let failed = runs
        .head(ORG_A, RUN_UNTOUCHED)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        (failed.state, failed.execution.reason_code),
        (
            RunState::Failed,
            Some(tam_storage::ImportReasonCode::ActivationExpired)
        ),
        "and the abandoned start is still failed with the code the console renders a next action \
         from, so the exemption has not been widened into never expiring anything"
    );
    Ok(())
}

/// A refusal the seller has not seen resolved survives the reclaim that
/// follows it, and is cleared by work rather than by a retry.
///
/// The incident this closes: every description the phone sent was refused by
/// the server, the device reported `interrupted` with `submission_failed`,
/// and then claimed again ten seconds later to resume. The claim wrote
/// `reported_stage`, `reason_code` and `reason` back to NULL unconditionally,
/// so each refusal survived for one cycle and the run read as an ordinary
/// resumed import. Four runs failed this way for twenty-four minutes with
/// nothing on the page naming a server refusal. The reclaim raises the fence
/// and must keep the narration; only accepted work retires it.
#[sqlx::test(migrations = "./migrations")]
async fn a_reclaim_keeps_the_refusal_until_work_retires_it(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    seed_org_a(&pool).await?;
    seed_devices(&pool, ORG_A).await;
    let runs = ImportRunRepo::new(pool.clone());
    let job = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, job))
        .await
        .expect("the run opens");
    let ClaimOutcome::Granted(first) = runs
        .claim(ORG_A, RUN_A, PHONE, false)
        .await
        .expect("the claim answers")
    else {
        panic!("an unclaimed run is claimable");
    };

    // The server refused the page. `interrupted` is not terminal: the run
    // stays open and the device intends to resume.
    assert_eq!(
        runs.report_progress(
            ORG_A,
            RUN_A,
            PHONE,
            &ProgressReport {
                attempt: first.attempt,
                stage: ImportStage::Interrupted,
                discovered: 13,
                processed: 4,
                reason_code: Some(tam_storage::ImportReasonCode::SubmissionFailed),
                reason: Some("the server would not take the description"),
            },
        )
        .await
        .expect("the report answers"),
        Some(RunState::Reading),
        "a refused page interrupts the run without settling it"
    );
    let reported = runs
        .head(ORG_A, RUN_A)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        (
            reported.execution.reported_stage,
            reported.execution.reason_code,
            reported.execution.reason.as_deref(),
            reported.execution.discovered,
            reported.execution.processed,
        ),
        (
            Some(ImportStage::Interrupted),
            Some(tam_storage::ImportReasonCode::SubmissionFailed),
            Some("the server would not take the description"),
            13,
            4,
        ),
        "the refusal is stored with the progress it stopped at"
    );
    let stopped_at = reported.execution.last_progress_at;

    // The device resumes: same owner, raised fence. Nothing about the
    // refusal has been resolved by asking for the run again.
    let ClaimOutcome::Granted(second) = runs
        .claim(ORG_A, RUN_A, PHONE, false)
        .await
        .expect("the claim answers")
    else {
        panic!("the owner may reclaim its own run");
    };
    assert!(
        second.attempt > first.attempt,
        "a reclaim is a new attempt, which is what retires the page still in flight"
    );
    let resumed = runs
        .head(ORG_A, RUN_A)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        (
            resumed.execution.reported_stage,
            resumed.execution.reason_code,
            resumed.execution.reason.as_deref(),
            resumed.execution.discovered,
            resumed.execution.processed,
        ),
        (
            Some(ImportStage::Interrupted),
            Some(tam_storage::ImportReasonCode::SubmissionFailed),
            Some("the server would not take the description"),
            13,
            4,
        ),
        "the reclaim raises the fence and changes nothing the seller is told: a run refused every \
         cycle must read as refused every cycle, not as a fresh resume"
    );

    // A renewal is a hold, not work. It may not retire the refusal, and it
    // may not claim progress on the device's behalf.
    assert!(
        runs.renew(ORG_A, RUN_A, PHONE, second.attempt)
            .await
            .expect("the renewal answers")
            .is_some(),
        "the current owner renews its own live hold"
    );
    let renewed = runs
        .head(ORG_A, RUN_A)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        (
            renewed.execution.reported_stage,
            renewed.execution.reason_code,
            renewed.execution.last_progress_at,
        ),
        (
            Some(ImportStage::Interrupted),
            Some(tam_storage::ImportReasonCode::SubmissionFailed),
            stopped_at,
        ),
        "holding the lease is not progress: the last thing that moved is still the page that was \
         refused"
    );

    // Work lands. A report that carries no refusal and advances the count is
    // what retires the narration -- and it must retire all of it, because a
    // code left behind would make a working import read as broken.
    assert_eq!(
        runs.report_progress(
            ORG_A,
            RUN_A,
            PHONE,
            &ProgressReport {
                attempt: second.attempt,
                stage: ImportStage::Reading,
                discovered: 13,
                processed: 5,
                reason_code: None,
                reason: None,
            },
        )
        .await
        .expect("the report answers"),
        Some(RunState::Reading),
    );
    let working = runs
        .head(ORG_A, RUN_A)
        .await
        .expect("the head reads")
        .expect("the run stands");
    assert_eq!(
        (
            working.execution.reported_stage,
            working.execution.reason_code,
            working.execution.reason.as_deref(),
            working.execution.processed,
        ),
        (Some(ImportStage::Reading), None, None, 5),
        "accepted work clears the stale refusal, stage, code and sentence together"
    );
    assert_eq!(
        working.state,
        RunState::Reading,
        "and the run was open throughout: nothing here settles it"
    );

    runs.report_progress(
        ORG_A,
        RUN_A,
        PHONE,
        &ProgressReport {
            attempt: second.attempt,
            stage: ImportStage::Interrupted,
            discovered: 13,
            processed: 5,
            reason_code: Some(tam_storage::ImportReasonCode::SubmissionFailed),
            reason: Some("the next page was not accepted"),
        },
    )
    .await
    .expect("the interruption is reported");

    // A page can advance the run without changing the device's counters.
    for (progressed, expected) in [
        (
            false,
            (
                Some(ImportStage::Interrupted),
                Some(tam_storage::ImportReasonCode::SubmissionFailed),
                Some("the next page was not accepted"),
            ),
        ),
        (true, (None, None, None)),
    ] {
        let mut tx = pool.begin().await?;
        sqlx::query("SELECT set_config('app.current_org', $1, true)")
            .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
            .execute(&mut *tx)
            .await?;
        tam_storage::note_contact(&mut tx, ORG_A, RUN_A, progressed)
            .await
            .expect("the page contact is recorded");
        tx.commit().await?;
        let head = runs
            .head(ORG_A, RUN_A)
            .await
            .expect("the head reads")
            .expect("the run stands");
        assert_eq!(
            (
                head.execution.reported_stage,
                head.execution.reason_code,
                head.execution.reason.as_deref(),
            ),
            expected,
            "contact alone preserves the interruption; accepted work retires it",
        );
    }
    Ok(())
}

// The publication provenance an upgrade has to supply, which is the half of
// migration 0080 a new column could not reach.
//
// `job.import_run_id` arrived with 0080 and is NULL on every publishing job a
// scheduled import minted before it. The deletion fence enumerates derived
// work through that column alone, so a database upgraded with a queued
// auto-publication in it holds a job whose only record of where it came from
// is the `auto_publish_run` receipt the pass wrote — tenant-qualified,
// durable, and unread. Deleting the import then fences the anchor, answers
// the seller, and leaves that publication claimable: a marketplace write for
// an import they were told is gone.
//
// The fixture is the upgrade itself. The historical rows are seeded against
// the schema as it stands applied, every later migration the repository
// carries is executed, and only then is the deletion asked for. With no
// repair in the tree that later set is empty, so the assertions below report
// the hole rather than fail to build.

/// The head migration the local database has applied, and therefore the
/// inclusive upper bound of the schema the historical rows are seeded
/// against.
const APPLIED_VERSION: i64 = 81;

/// A publishing job of the shape a pre-0080 scheduler pass left: queued, with
/// an item to claim, naming no import at all.
const HISTORICAL_PUBLICATION: JobId = JobId(Uuid([0x9C; 16]));
const HISTORICAL_ITEM: tam_domain::JobItemId = tam_domain::JobItemId(Uuid([0x9D; 16]));
const HISTORICAL_MAPPING: tam_types::MappingId = tam_types::MappingId(Uuid([0x9E; 16]));

async fn schema_as_applied(pool: &PgPool) -> Result<(), sqlx::Error> {
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.version <= APPLIED_VERSION)
    {
        pool.execute(&*migration.sql).await?;
    }
    Ok(())
}

/// Every migration the repository carries beyond the applied head, which is
/// what an operator's upgrade runs over the rows seeded above.
async fn upgrade_past_applied(pool: &PgPool) -> Result<(), sqlx::Error> {
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.version > APPLIED_VERSION)
    {
        pool.execute(&*migration.sql).await?;
    }
    Ok(())
}

/// The resource the historical publication sends, and the mapping its item
/// names.
async fn seed_published_resource(pool: &PgPool) -> Result<(), tam_storage::StorageError> {
    tam_storage::ProductRepo::new(pool.clone())
        .insert(ORG_A, &minimal_product(), NOW)
        .await?;
    tam_storage::MappingRepo::new(pool.clone())
        .insert(
            ORG_A,
            &tam_domain::Mapping {
                id: HISTORICAL_MAPPING,
                org: ORG_A,
                product: PRODUCT_1,
                inventory: InventoryId::Tes,
                binding: tam_domain::Binding::Unbound,
                policies: tam_domain::FieldPolicies {
                    title: tam_domain::FieldPolicy::Managed,
                    description: tam_domain::FieldPolicy::Managed,
                    price: tam_domain::FieldPolicy::Managed,
                    taxonomy: tam_domain::FieldPolicy::Managed,
                    grades: tam_domain::FieldPolicy::Managed,
                    files: tam_domain::FieldPolicy::Managed,
                },
                price_rule: tam_types::PriceRule::Explicit(tam_types::PriceIntent::Free),
                publish: tam_domain::PublishMode::DryRun,
                lifecycle: tam_marketplace::RemoteLifecycle::Absent,
            },
            0,
            NOW,
        )
        .await?;
    Ok(())
}

// The current enqueue path reads policy tables that do not exist at schema 81.
// Seed the historical rows directly, before exercising the real upgrade.
async fn seed_historical_publication(pool: &PgPool) -> Result<(), sqlx::Error> {
    let db = |id: Uuid| uuid::Uuid::from_bytes(id.0);
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(as_text(ORG_A.0))
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO job \
         (org_id, id, inventory, marketplace, created_at, request_idempotency_key, \
          actor_kind, actor_id) \
         VALUES ($1, $2, 'tes', 'tes', to_timestamp($3::double precision / 1000), \
                 $4, 'system', 'scheduler')",
    )
    .bind(db(ORG_A.0))
    .bind(db(HISTORICAL_PUBLICATION.0))
    .bind(NOW.0)
    .bind(db(tam_storage::job_request_key(
        RUN_A,
        "publish:historical",
    )))
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO job_item \
         (org_id, id, job_id, mapping_id, idempotency_key, state, created_at, \
          operation, marketplace) \
         SELECT $1, $2, $3, $4, $5, 'queued', to_timestamp($6::double precision / 1000), \
                'create', m.marketplace \
         FROM mapping m WHERE m.org_id = $1 AND m.id = $4",
    )
    .bind(db(ORG_A.0))
    .bind(db(HISTORICAL_ITEM.0))
    .bind(db(HISTORICAL_PUBLICATION.0))
    .bind(db(HISTORICAL_MAPPING.0))
    .bind(db(Uuid([0x9F; 16])))
    .bind(NOW.0)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// Deleting an upgraded database's import fences the publication it produced
/// before the provenance column existed.
#[sqlx::test(migrations = false)]
async fn an_upgrade_fences_a_historical_publication_with_its_import(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    schema_as_applied(&pool).await?;
    seed_org_a(&pool).await?;
    seed_published_resource(&pool)
        .await
        .expect("the publication resource seeds");

    let runs = ImportRunRepo::new(pool.clone());
    let anchor = anchor(&pool, ORG_A, RUN_A, InventoryId::Tes).await;
    runs.create(ORG_A, &new_run(RUN_A, InventoryId::Tes, anchor))
        .await
        .expect("the historical import opens");

    let jobs = tam_storage::JobRepo::new(pool.clone());
    seed_historical_publication(&pool).await?;
    assert!(
        tam_storage::SyncSettingRepo::new(pool.clone())
            .record_auto_publish(
                ORG_A,
                RUN_A,
                PRODUCT_1,
                InventoryId::Tes,
                HISTORICAL_PUBLICATION,
                NOW,
            )
            .await
            .expect("the receipt writes"),
        "the receipt is the ownership a database upgraded from before 0080 holds"
    );

    upgrade_past_applied(&pool).await?;
    assert_eq!(
        tam_storage::JobReadRepo::new(pool.clone())
            .snapshot(ORG_A, HISTORICAL_PUBLICATION)
            .await
            .expect("the upgraded publication reads")
            .expect("the publication is still visible before deletion")
            .counts
            .queued,
        1,
        "the upgrade preserves the historical queued write until its import is deleted"
    );

    runs.delete(
        ORG_A,
        RUN_A,
        tam_types::Stamp {
            at: NOW,
            actor: tam_types::Actor::System(tam_types::SystemComponent::Import),
        },
    )
    .await
    .expect("the deletion answers")
    .expect("the import is this tenant's");

    assert!(
        jobs.deletion_status(ORG_A, HISTORICAL_PUBLICATION)
            .await
            .expect("the publication's deletion state reads")
            .is_some(),
        "the publication this import produced is fenced with it: its only link is the \
         auto_publish_run receipt, and a fence that reads the provenance column alone leaves \
         the job claimable after the seller was told the import is gone"
    );
    let outstanding = match tam_storage::JobReadRepo::new(pool.clone())
        .snapshot(ORG_A, HISTORICAL_PUBLICATION)
        .await
        .expect("the publication reads")
    {
        // A fenced job whose every item settled is tombstoned, and a
        // tombstone has no snapshot at all: nothing is left to claim.
        None => 0,
        Some(snapshot) => snapshot.counts.queued,
    };
    assert_eq!(
        outstanding, 0,
        "and its queued marketplace write is cancelled rather than left for the next device to \
         claim"
    );
    Ok(())
}
