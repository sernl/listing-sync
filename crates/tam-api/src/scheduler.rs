//! The pass: one minute's worth of both clocks, for every tenant.
//!
//! It runs inside the serving process, beside the import sweep, and as
//! `tam_app` with one tenant pinned per organisation. That is the whole
//! reason it is not in `tam-worker`: that process connects as `tam_engine`,
//! which by migration 0025's deliberate rule holds no grant on the tables a
//! tick writes, and it carries no `AppState` -- no blob store, no
//! entitlement, none of the handlers a commit reuses. A pass that had to
//! re-implement `commit_one` under a role that cannot see a tenant's rows
//! would be a second import.
//!
//! Every tenant is its own error scope. One organisation whose Tes
//! connection was revoked, whose zone name no longer parses, or whose
//! database row is corrupt must not stop the next organisation's Friday
//! drop, so a tenant's failure is collected into the report and the loop
//! carries on. The report is data rather than a log line for the same reason
//! the clock is: this crate opens no socket and writes no stdio, and the
//! serving binary prints what the pass returns.
//!
//! What one pass does, in order, per tenant:
//!
//!  1. reads the plan, because every step below is gated on a capability;
//!  2. fires every schedule whose latest tick has passed and is unrecorded;
//!  3. mints an import run for the shop whose interval has elapsed, unless a
//!     run is already open -- there is one open run per organisation;
//!  4. commits a scheduled run the device has finished reading;
//!  5. publishes what that commit created, through the seller's own rules.
//!
//! Steps 2 and 3 both write; 4 and 5 finish what 3 started on an earlier
//! pass. Nothing here waits on a device: the run is the rendezvous, and the
//! device finds it through `GET /{version}/devices/{device}/import/open`.

use tam_marketplace::ListingState;
use tam_storage::{
    due_tick, job_request_key, EntitlementRepo, ImportRunRepo, JobReadRepo, MappingRepo,
    NewImportRun, NewJobItem, ProductRepo, RunKind, RunOpening, RunState, ScheduleMember,
    ScheduleOutcome, ScheduleRecord, ScheduleRepo, ScheduleRunWrite, SyncIntent, SyncRequestRepo,
    SyncSettingRepo,
};
use tam_types::{
    Actor, InventoryId, JobId, MappingId, OrgId, PriceIntent, ProductId, SystemComponent,
    Timestamp, TransportClass, Uuid,
};

use crate::catalogue::unbound_mapping;
use crate::entitlement::Entitlement;
use crate::error::APIError;
use crate::jobs::{mint_job, new_items, storage_fault};
use crate::AppState;

/// What one pass did, for the serving binary's own log line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassReport {
    pub tenants: u32,
    /// Schedule firings materialised, counted per marketplace.
    pub ticks: u32,
    /// Jobs minted, by a tick or by a rule.
    pub jobs: u32,
    /// Import runs opened for a due shop.
    pub pulls: u32,
    /// Resources a scheduled run committed into the catalogue.
    pub committed: u32,
    /// Pulled resources a rule's template filled fields on.
    pub filled: u32,
    /// Manual runs no device picked up inside the activation window.
    pub expired: u32,
    /// Runs whose owner stopped answering, marked interrupted so the console
    /// stops saying a dead device is reading.
    pub interrupted: u32,
    /// One tenant's failure, named. The pass carries on past each of these.
    pub failures: Vec<String>,
}

impl PassReport {
    /// Whether this pass did anything worth a line on the log.
    #[must_use]
    pub const fn eventful(&self) -> bool {
        self.ticks > 0
            || self.jobs > 0
            || self.pulls > 0
            || self.committed > 0
            || self.filled > 0
            || self.expired > 0
            || self.interrupted > 0
            || !self.failures.is_empty()
    }
}

/// One pass over every tenant.
///
/// # Errors
///
/// Only the tenant enumeration, which is one unpinned read of
/// `organisation`: a pass that cannot list tenants has nothing to iterate and
/// is a fault rather than one tenant's bad luck.
pub async fn pass(state: &AppState, now: Timestamp) -> Result<PassReport, APIError> {
    let tenants = SyncRequestRepo::new(state.pool.clone())
        .tenants()
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let mut report = PassReport::default();
    for org in tenants {
        report.tenants = report.tenants.saturating_add(1);
        if let Err(failure) = tenant_pass(state, org, now, &mut report).await {
            report
                .failures
                .push(format!("{}: {failure}", org.0.to_hyphenated()));
        }
    }
    Ok(report)
}

async fn tenant_pass(
    state: &AppState,
    org: OrgId,
    now: Timestamp,
    report: &mut PassReport,
) -> Result<(), APIError> {
    let grant = EntitlementRepo::new(state.pool.clone())
        .current(org, now)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let caps = Entitlement::of(grant).caps;

    if caps.scheduling {
        let schedules = ScheduleRepo::new(state.pool.clone())
            .list(org)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        for schedule in &schedules {
            if let Some(tick) = due_tick(schedule, now) {
                fire(state, org, schedule, tick, now, report).await?;
            }
        }
    }

    // Maintenance first, so a run whose device died is interrupted rather
    // than reported as reading for one more minute, and a manual start
    // nobody answered reaches an actionable failure rather than waiting
    // forever.
    maintain(state, org, report).await?;

    if caps.sync_pull_interval_secs.is_some() {
        pull(state, org, now, report).await?;
    }
    // Finishing a run is not gated on the pull capability. A tenant whose
    // plan lapsed between the pull and the read still has resources their own
    // machine described sitting in a run, and abandoning those would lose
    // work they already did.
    finish(state, org, now, caps.auto_publish_rules, report).await?;
    Ok(())
}

// ------------------------------------------------------------- schedules

/// Materialises one schedule's tick: one job per marketplace, one
/// `schedule_run` row per member per marketplace.
#[expect(
    clippy::too_many_arguments,
    reason = "a tick is the tenant, the schedule, the instant it belongs to and the instant it \
              ran; the report is the pass's own accumulator"
)]
async fn fire(
    state: &AppState,
    org: OrgId,
    schedule: &ScheduleRecord,
    tick: Timestamp,
    now: Timestamp,
    report: &mut PassReport,
) -> Result<(), APIError> {
    let repo = ScheduleRepo::new(state.pool.clone());
    let members = repo
        .members(org, schedule)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    for inventory in &schedule.inventories {
        // Already done, on an earlier pass that died before marking the
        // schedule. The row is the fence; this is the cheap read of it.
        if repo
            .tick_recorded(org, schedule.id, tick, *inventory)
            .await
            .map_err(|error| storage_fault(state, &error))?
        {
            continue;
        }
        let rows = send(state, org, schedule, *inventory, &members, tick, now).await?;
        if rows
            .iter()
            .any(|row| matches!(row.outcome, ScheduleOutcome::Sent(_)))
        {
            report.jobs = report.jobs.saturating_add(1);
        }
        repo.record_run(org, schedule.id, tick, &rows, now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        report.ticks = report.ticks.saturating_add(1);
    }
    // Marked even where every marketplace resolved nothing, which is what
    // stops a tick with no members being recomputed as due for ever: there is
    // no `schedule_run` row to read in that case, and `last_run_at` is the
    // only thing that can say the minute was reached.
    repo.record_run(org, schedule.id, tick, &[], now)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(())
}

/// One marketplace's share of one tick: the mappings ensured, the intent
/// lowered per member, one job minted, and a row per member either way.
#[expect(
    clippy::too_many_arguments,
    reason = "see `fire`: the marketplace and the resolved members are what this adds, and \
              both are the loop's own variables rather than a value worth naming"
)]
async fn send(
    state: &AppState,
    org: OrgId,
    schedule: &ScheduleRecord,
    inventory: InventoryId,
    members: &[ScheduleMember],
    tick: Timestamp,
    now: Timestamp,
) -> Result<Vec<ScheduleRunWrite>, APIError> {
    let ids: Vec<ProductId> = members.iter().map(|member| member.product).collect();
    let mappings = MappingRepo::new(state.pool.clone());
    let heads = mappings
        .heads_for_products(org, inventory, &ids)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let prices = prices_of(state, org).await?;
    // What a create on this marketplace needs, read once for the tick's
    // members rather than per member. Facts only: whether the resource holds
    // a payload, where those bytes come from, whether its grant is declared
    // and which axes it has settled. The policy over them is
    // `catalogue::creation_blocked`, which is the same decision the seller's
    // own "add this marketplace" answers with.
    //
    // Checked before the mint rather than after it, and that is the point: a
    // member this marketplace will refuse used to get a mapping and a job
    // item anyway, and a fileless one used to yield no seed and no
    // explanation. Now it gets neither, and the run row says why.
    let facts = ProductRepo::new(state.pool.clone())
        .creation_facts(org, &ids, inventory)
        .await
        .map_err(|error| storage_fault(state, &error))?;

    let mut rows: Vec<ScheduleRunWrite> = Vec::new();
    let mut chosen: Vec<(ProductId, MappingId)> = Vec::new();
    for member in members {
        let head = heads.iter().find(|head| head.product == member.product);
        if let Some(head) = head.filter(|head| head.binding_state == "bound") {
            // Already listed here. Without the republish rule there is
            // nothing to send, and it is not a refusal either: recording a
            // skip per member per day would fill the runs list with rows
            // saying that a listing still exists.
            if !schedule.republish_on_update {
                continue;
            }
            // And with it, only where the resource has actually moved since
            // the listing was last written. `product.updated_at` is bumped by
            // every write and by every file change, and `mapping.updated_at`
            // by every write to the listing, so the comparison is the drift
            // signal and nothing has to be stored to compute it.
            if member.updated_at.0 <= head.updated_at.0 {
                continue;
            }
        }
        // A member the marketplace would refuse is skipped before anything is
        // minted for it. An already-bound republish is past this: its listing
        // exists, so what a create would need is not the question being asked.
        if head.is_none_or(|head| head.binding_state != "bound") {
            let blocked = facts
                .iter()
                .find(|held| held.product == member.product)
                .and_then(|held| crate::catalogue::creation_blocked(held, inventory));
            if let Some(blocked) = blocked {
                rows.push(skipped(member.product, inventory, &blocked.reason()));
                continue;
            }
        }
        let mapping = if let Some(head) = head {
            head.id
        } else {
            // The same mapping `POST /{version}/products/{product}/mappings`
            // writes when a seller adds a marketplace by hand, so a tick that
            // then fails leaves them the mapping they would have got by
            // ticking it rather than an artefact of a half-run drop.
            let price = prices
                .iter()
                .find(|(product, _)| *product == member.product)
                .map_or(PriceIntent::Free, |(_, price)| *price);
            let id = MappingId(fresh_uuid());
            let minted = unbound_mapping(org, member.product, inventory, id, price);
            crate::migrations::mint(state, &mappings, &minted, inventory, now).await?
        };
        chosen.push((member.product, mapping));
    }
    if chosen.is_empty() {
        return Ok(rows);
    }

    let mapping_ids: Vec<MappingId> = chosen.iter().map(|(_, mapping)| *mapping).collect();
    // A write already owed on the mapping — a create the seller confirmed
    // from Migrations, a schedule's own earlier tick, a cross-list — is not
    // sent again. The item ledger would refuse the identical key with a 409
    // and take the whole tick with it, so the check is made here, per member,
    // and the member is recorded as on its way instead.
    let queued = mappings
        .with_open_items(org, &mapping_ids)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let seeds = JobReadRepo::new(state.pool.clone())
        .mapping_seeds(org, inventory, &mapping_ids)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let to = match schedule.intent {
        SyncIntent::Draft => ListingState::Draft,
        SyncIntent::Live => ListingState::Live,
    };
    let job = JobId(fresh_uuid());
    let mut items: Vec<NewJobItem> = Vec::new();
    let mut sent: Vec<ProductId> = Vec::new();
    for (product, mapping) in &chosen {
        if queued.contains(mapping) {
            rows.push(skipped(
                *product,
                inventory,
                "already on its way; a write to this listing is still queued",
            ));
            continue;
        }
        let Some(seed) = seeds.iter().find(|seed| seed.mapping == *mapping) else {
            // `mapping_seeds` inner-joins the payload file, so a resource
            // with no file yields no seed. Silent at `POST /{version}/jobs`,
            // where the seller sees a short list; a schedule has to say so,
            // because nobody is watching the tick.
            rows.push(skipped(
                *product,
                inventory,
                "this resource has no file yet, so there is nothing to send",
            ));
            continue;
        };
        match tam_storage::lower(to, inventory, seed) {
            Ok(operations) => {
                items.extend(new_items(org, inventory, job, seed, operations));
                sent.push(*product);
            }
            // The refusal sentence, verbatim: an uncaptured Tes edit, a
            // lifecycle nobody observed, a create still in flight. Recorded
            // against the member rather than failing the tick, so one
            // unrevisable listing costs the seller one resource.
            Err(refusal) => rows.push(skipped(*product, inventory, &refusal.to_string())),
        }
    }
    if items.is_empty() {
        return Ok(rows);
    }
    let created = mint_job(
        state,
        org,
        job,
        inventory,
        // Nobody pressed anything. The schedule is the seller's decision and
        // the minute is not, which is exactly what `actor_kind = 'system'`
        // records.
        Actor::System(SystemComponent::Scheduler),
        now,
        // v5 over the schedule, the tick and the marketplace: a pass
        // repeating a minute mints the same key, and `create_with_request_key`
        // answers the first job rather than a second one.
        job_request_key(schedule.id, &leg(tick, inventory)),
        &items,
    )
    .await?;
    for product in sent {
        rows.push(ScheduleRunWrite {
            product,
            inventory,
            outcome: ScheduleOutcome::Sent(created.job),
        });
    }
    Ok(rows)
}

/// The request key's second half: the tick and the marketplace, so one
/// schedule's two marketplaces and its two minutes are four keys.
fn leg(tick: Timestamp, inventory: InventoryId) -> String {
    format!("schedule:{}:{inventory:?}", tick.0)
}

fn skipped(product: ProductId, inventory: InventoryId, reason: &str) -> ScheduleRunWrite {
    ScheduleRunWrite {
        product,
        inventory,
        outcome: ScheduleOutcome::Skipped(reason.to_owned()),
    }
}

// ------------------------------------------------------------- sync pulls

/// Opens an import run for every shop whose interval has elapsed and has no
/// run open.
///
/// Per source since migration 0074: a tenant syncing two shops no longer
/// reads them on consecutive passes, because the fence that made that
/// necessary was organisation-wide and is now per shop. One device may still
/// only serve one at a time, and that is the device's own decision rather
/// than a fence here.
async fn pull(
    state: &AppState,
    org: OrgId,
    now: Timestamp,
    report: &mut PassReport,
) -> Result<(), APIError> {
    let runs = ImportRunRepo::new(state.pool.clone());
    let settings = SyncSettingRepo::new(state.pool.clone());
    let due: Vec<InventoryId> = settings
        .list(org)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .into_iter()
        .filter(|setting| setting.due(now))
        .map(|setting| setting.inventory)
        .filter(|inventory| {
            inventory.marketplace().transport_class() == TransportClass::SellerDevice
        })
        .collect();
    for inventory in due {
        if runs
            .open_for_source(org, inventory)
            .await
            .map_err(|error| storage_fault(state, &error))?
            .is_some()
        {
            continue;
        }
        // Every file a run imports is fetched back through the seller's own
        // connection, so a shop with no linked one has nothing an import
        // could keep. Skipped rather than failed: a revoked connection is the
        // seller's to restore and is not this pass's fault.
        if crate::import::source_connection(state, org, inventory, now)
            .await
            .is_err()
        {
            continue;
        }
        let run = fresh_uuid();
        let anchor = crate::import_runs::anchor_job(state, org, run, inventory, now).await?;
        let opening = runs
            .create(
                org,
                &NewImportRun {
                    id: run,
                    kind: RunKind::Marketplace,
                    source: Some(inventory),
                    batch_id: None,
                    target: None,
                    anchor_job: anchor,
                    created_at: now,
                    scheduled: true,
                    // The interval is this run's identity; there is no client
                    // retry to deduplicate and no seller pressing Retry.
                    start_key: None,
                    retry_of: None,
                },
            )
            .await
            .map_err(|error| storage_fault(state, &error))?;
        match opening {
            RunOpening::Opened => {
                // Marked from the instant the run opened rather than when it
                // finishes: the interval measures how often the seller's
                // machine is asked, and a shop that takes an hour to read
                // must not be asked again the minute it lands.
                settings
                    .mark_pulled(org, inventory, now)
                    .await
                    .map_err(|error| storage_fault(state, &error))?;
                report.pulls = report.pulls.saturating_add(1);
            }
            // A run for this shop opened between the read above and this
            // write. The index is the authority; this pass does nothing more
            // for that shop and carries on to the next.
            RunOpening::AlreadyOpen(_) | RunOpening::KeySpent { .. } => {}
        }
    }
    Ok(())
}

// -------------------------------------------------------- finishing a run

/// Creates what every authorised run of this tenant still owes, then
/// publishes what a scheduled one created.
///
/// Authorisation rather than state is the predicate, and it is what removes
/// the browser's commit loop: a seller who confirmed and closed the tab, and
/// a scheduled run carrying its own approved rule, are the same work here. A
/// run whose descriptions merely finished is not drained, which is what stops
/// an unconfirmed manual import creating anything.
async fn finish(
    state: &AppState,
    org: OrgId,
    now: Timestamp,
    rules_held: bool,
    report: &mut PassReport,
) -> Result<(), APIError> {
    let runs = ImportRunRepo::new(state.pool.clone());
    let drainable = runs
        .drainable(org)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    for head in &drainable {
        // By kind, because the two sources' remaining work is not the same
        // work. A marketplace run's items hold a device's `ObservedResource`
        // and name a shop; a spreadsheet run's hold the seller's own draft
        // and name a batch, so handing one to the other's commit would try to
        // decode a draft as a marketplace read and fail the tenant's whole
        // drain with it.
        let created = match head.kind {
            tam_storage::RunKind::Marketplace => {
                crate::import_runs::drain_run(state, org, head).await?
            }
            tam_storage::RunKind::Spreadsheet => {
                crate::import_batch::drain_batch_run(state, org, head).await?
            }
        };
        report.committed = report.committed.saturating_add(created);
        // Publishing follows a scheduled run's own rule. A manual import
        // drafts nowhere, which is the whole of what phase 2 means by
        // "nothing drafted anywhere".
        if head.scheduled && rules_held {
            publish(state, org, head, now, report).await?;
        }
    }
    Ok(())
}

/// The maintenance half of a pass: activations that expired and leases that
/// lapsed.
///
/// Here rather than in a loop of its own, because this is the existing
/// per-tenant pass and the facts it writes are the ones the console reads. A
/// scheduled run is never expired for waiting: it truthfully awaits a device.
async fn maintain(state: &AppState, org: OrgId, report: &mut PassReport) -> Result<(), APIError> {
    let runs = ImportRunRepo::new(state.pool.clone());
    let expired = runs
        .expire_activations(org, ACTIVATION_EXPIRED)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    for run in &expired {
        if let Some(head) = runs
            .head(org, *run)
            .await
            .map_err(|error| storage_fault(state, &error))?
        {
            crate::import_runs::settled_event(state, org, &head, RunState::Failed).await?;
            report.expired = report.expired.saturating_add(1);
        }
    }
    let interrupted = runs
        .interrupt_stale_leases(org)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    report.interrupted = report
        .interrupted
        .saturating_add(u32::try_from(interrupted.len()).unwrap_or(u32::MAX));
    // The console reads these from the ledger like everything else, so the
    // rows this pass moved are announced: a run whose phone stopped answering
    // shows as interrupted on a page nobody reloaded, and an expired manual
    // start shows its actionable failure.
    for run in &interrupted {
        if let Some(head) = runs
            .head(org, *run)
            .await
            .map_err(|error| storage_fault(state, &error))?
        {
            let counts = runs
                .counts(org, *run)
                .await
                .map_err(|error| storage_fault(state, &error))?;
            crate::import_runs::progress_event(state, org, &head, counts).await?;
        }
    }
    Ok(())
}

/// What a run that nobody picked up is told, in the seller's own words.
///
/// Stated rather than left to a code: the console renders this line, and a
/// failure with no sentence reads as something having gone wrong on our side
/// rather than as an import no device answered.
const ACTIVATION_EXPIRED: &str =
    "no device picked this import up, so it was stopped. Open the app on the phone that has \
     your shop signed in, then start it again.";

/// Sends what a pull brought in on to the seller's chosen marketplaces,
/// filling each resource from the rule's template first.
///
/// A freshly pulled resource is bound on its source alone and carries what
/// that shop held, which is never the whole form: no marketplace answers our
/// copyright attestation, our tax code, our formats or our details. So the
/// rule's template fills those — fill-empty, so nothing the pull did carry is
/// overwritten — and the fill happens once per resource, before the target
/// loop, because the fields it writes live on the product and its sidecar
/// rather than on any one marketplace's mapping.
///
/// Before the seed rather than after: `mapping_seeds` is what the lowering
/// consumes, so a template applied after it would reach the next sync and not
/// this one.
///
/// A resource the template cannot fill — a merge the create form's own rules
/// refuse — is published unfilled rather than failing the tenant's pass. The
/// seller sees it on the resource page with the fields still blank, which is
/// the state it would have been in with no template at all.
async fn publish(
    state: &AppState,
    org: OrgId,
    head: &tam_storage::ImportRunHead,
    now: Timestamp,
    report: &mut PassReport,
) -> Result<(), APIError> {
    let Some(source) = head.source else {
        return Ok(());
    };
    let settings = SyncSettingRepo::new(state.pool.clone());
    let setting = settings
        .list(org)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .into_iter()
        .find(|setting| setting.inventory == source);
    let targets: Vec<InventoryId> = setting
        .as_ref()
        .map(|setting| setting.publish_to.clone())
        .unwrap_or_default();
    if targets.is_empty() {
        return Ok(());
    }
    let products = ImportRunRepo::new(state.pool.clone())
        .imported_products(org, head.id)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    if products.is_empty() {
        return Ok(());
    }
    let mappings = MappingRepo::new(state.pool.clone());
    if let Some(draft) =
        rule_template(state, org, setting.and_then(|setting| setting.template)).await?
    {
        for product in &products {
            let bound = mappings
                .list_for_product(org, *product)
                .await
                .map_err(|error| storage_fault(state, &error))?;
            let filled = crate::template_apply::fill_from_template(
                state, org, *product, &draft, &bound, now,
            )
            .await;
            if let Ok(true) = filled {
                report.filled = report.filled.saturating_add(1);
            }
        }
    }
    let prices = prices_of(state, org).await?;
    let reads = JobReadRepo::new(state.pool.clone());
    for target in targets {
        let heads = mappings
            .heads_for_products(org, target, &products)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        for product in &products {
            if settings
                .auto_published(org, head.id, *product, target)
                .await
                .map_err(|error| storage_fault(state, &error))?
            {
                continue;
            }
            let existing = heads.iter().find(|head| head.product == *product);
            // Already listed there. A rule publishes what a pull brought in;
            // a resource the seller had already cross-listed is not this
            // rule's to rewrite.
            if existing.is_some_and(|head| head.binding_state == "bound") {
                continue;
            }
            let mapping = if let Some(head) = existing {
                head.id
            } else {
                let price = prices
                    .iter()
                    .find(|(held, _)| held == product)
                    .map_or(PriceIntent::Free, |(_, price)| *price);
                let id = MappingId(fresh_uuid());
                let minted = unbound_mapping(org, *product, target, id, price);
                crate::migrations::mint(state, &mappings, &minted, target, now).await?
            };
            // A create already queued on the target — by a migration, a
            // schedule or the seller — is left to land; a second identical
            // item would be refused by the ledger and fail the tenant's pass.
            if !mappings
                .with_open_items(org, &[mapping])
                .await
                .map_err(|error| storage_fault(state, &error))?
                .is_empty()
            {
                continue;
            }
            let seeds = reads
                .mapping_seeds(org, target, &[mapping])
                .await
                .map_err(|error| storage_fault(state, &error))?;
            let Some(seed) = seeds.first() else {
                continue;
            };
            // Live, which is what a rule is for: a seller who asked for a
            // pulled resource to be published on Tes did not ask for a draft
            // to go and finish by hand.
            let Ok(operations) = tam_storage::lower(ListingState::Live, target, seed) else {
                continue;
            };
            let job = JobId(fresh_uuid());
            let items = new_items(org, target, job, seed, operations);
            if items.is_empty() {
                continue;
            }
            let created = mint_job(
                state,
                org,
                job,
                target,
                Actor::System(SystemComponent::Scheduler),
                now,
                // v5 over the run, the resource and the target, which is the
                // idempotency the design states for a rule.
                job_request_key(
                    head.id,
                    &format!("publish:{}:{target:?}", product.0.to_hyphenated()),
                ),
                &items,
            )
            .await?;
            if settings
                .record_auto_publish(org, head.id, *product, target, created.job, now)
                .await
                .map_err(|error| storage_fault(state, &error))?
            {
                report.jobs = report.jobs.saturating_add(1);
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ shared

/// The draft a rule's template holds, or `None` where the rule names none.
///
/// A template deleted between the rule being written and this pass reads as
/// no template at all, which is the column's own `ON DELETE SET NULL`: the
/// rule keeps publishing and fills nothing, which is what it did before a
/// template could be named. A stored document that will not read back as a
/// draft is read the same way rather than failing the pass, because a
/// template nobody can parse must not stop a tenant's shop being published.
async fn rule_template(
    state: &AppState,
    org: OrgId,
    template: Option<Uuid>,
) -> Result<Option<crate::product::DraftInput>, APIError> {
    let Some(id) = template else {
        return Ok(None);
    };
    Ok(tam_storage::ResourceTemplateRepo::new(state.pool.clone())
        .get(org, id)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .and_then(|held| serde_json::from_value(held.draft).ok()))
}

/// Every catalogue price, for the mappings a tick or a rule mints.
///
/// One read per tenant pass rather than one per resource: a mapping's
/// `price_rule` is the product's own price at mint time, which is what
/// `add_mapping` writes, and reading it per member would be a statement per
/// resource per marketplace.
async fn prices_of(
    state: &AppState,
    org: OrgId,
) -> Result<Vec<(ProductId, PriceIntent)>, APIError> {
    Ok(ProductRepo::new(state.pool.clone())
        .list(org)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .into_iter()
        .map(|summary| (summary.id, summary.price))
        .collect())
}

fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}
