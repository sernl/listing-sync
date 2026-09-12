//! Marketplace Sync's own two routes: how often each shop is re-read, and
//! where a resource it brings in is published afterwards.
//!
//! The list is padded from the closed marketplace set rather than from the
//! stored rows, because the page draws a card per marketplace whether or not
//! the seller has ever touched one: a shop absent from the list tells them
//! nothing about whether it could be synced. The floor travels with each row
//! as `minimum_secs` for the same reason -- the control disables the options
//! below it and needs the number to do that, and `null` is a plan that holds
//! no sync at all.
//!
//! Nothing here reads a clock. The interval is stored and the comparison is
//! the pass's; a route that decided a shop was due would be a second clock.

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{ResourceTemplateRepo, SyncSettingRecord, SyncSettingRepo};
use tam_types::{InventoryId, Timestamp, TransportClass, Uuid};

use crate::entitlement::feature_refusal;
use crate::error::APIError;
use crate::jobs::{storage_fault, validation};
use crate::migrations::name_of;
use crate::vocabulary::parse_inventory;
use crate::{AppState, OrgContext};

/// The console's own sentence for a plan that pulls nothing, matching
/// `entitlement.ts::featureReason('sync')`.
const NO_SYNC: &str = "Your plan does not include marketplace sync. Upgrade to pull changes from \
                       your marketplaces.";

/// And for the rules, which are the second capability.
const NO_RULES: &str = "Your plan does not include publishing a pulled resource automatically. \
                        Upgrade to use it.";

/// What a seller may choose: six-hourly, daily, weekly.
///
/// A closed set rather than a free number, because it is a radio group in the
/// console and a free interval is a support question about why a shop is read
/// every ninety seconds. The plan's floor then removes the ones a plan does
/// not reach, which is why both exist: the set is what the control offers and
/// the floor is what the plan allows of it.
pub const INTERVAL_CHOICES: [u32; 3] = [6 * 3_600, 24 * 3_600, 7 * 24 * 3_600];

/// The interval a marketplace the seller has never configured is offered at.
///
/// Daily rather than the shortest, because a shop is read by the seller's own
/// machine and the default should be the one that costs them least.
const DEFAULT_INTERVAL_SECS: u32 = 24 * 3_600;

// ------------------------------------------------------------------- wire

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSettingView {
    pub inventory: InventoryId,
    pub enabled: bool,
    pub interval_secs: u32,
    /// The shortest interval this plan reaches. `null` is a plan that holds
    /// no sync at all, which is what the console's own gate reads.
    pub minimum_secs: Option<u32>,
    pub last_pull_at: Option<Timestamp>,
    pub publish_to: Vec<InventoryId>,
    /// The template a rule fills a pulled resource from, or `null` where the
    /// seller has named none.
    ///
    /// A pull carries what the source marketplace held, which is never the
    /// whole form: the copyright attestation, the tax code, the formats and
    /// the details are ours to ask for and no shop answers them. The template
    /// is what fills those, and only where the pull left them empty.
    pub template_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSettingsView {
    pub marketplaces: Vec<SyncSettingView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSettingBody {
    pub enabled: bool,
    pub interval_secs: u32,
    #[serde(default)]
    pub publish_to: Vec<InventoryId>,
    /// The template a rule fills from. Absent and null are the same answer
    /// here, unlike on the template routes: the picker has a "no template"
    /// option and sends it.
    #[serde(default)]
    pub template_id: Option<Uuid>,
}

// ---------------------------------------------------------------- handlers

pub(crate) async fn list_settings(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<SyncSettingsView>, APIError> {
    let minimum = context.entitlement.caps.sync_pull_interval_secs;
    let stored = SyncSettingRepo::new(state.pool.clone())
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let marketplaces = InventoryId::ALL
        .into_iter()
        // Only the shops a device reads. An official-API marketplace is
        // imported on our own infrastructure and has no pull for a seller to
        // set a cadence on, which is `create_run`'s own refusal stated as an
        // absence rather than as a card that cannot be switched on.
        .filter(|inventory| {
            inventory.marketplace().transport_class() == TransportClass::SellerDevice
        })
        .map(|inventory| {
            stored
                .iter()
                .find(|row| row.inventory == inventory)
                .map_or_else(
                    || SyncSettingView {
                        inventory,
                        enabled: false,
                        interval_secs: DEFAULT_INTERVAL_SECS,
                        minimum_secs: minimum,
                        last_pull_at: None,
                        publish_to: Vec::new(),
                        template_id: None,
                    },
                    |row| view_of(row, minimum),
                )
        })
        .collect();
    Ok(Json(SyncSettingsView { marketplaces }))
}

pub(crate) async fn update_setting(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, inventory)): Path<(String, String)>,
    Json(body): Json<SyncSettingBody>,
) -> Result<Json<SyncSettingView>, APIError> {
    let inventory = parse_inventory(&inventory)
        .ok_or_else(|| validation("that is not a marketplace this server knows"))?;
    let caps = context.entitlement.caps;
    // The capability before the value: a plan that pulls nothing is not
    // choosing badly between six hours and a day, it does not pull at all,
    // and the two refusals send the console to different places.
    let Some(minimum) = caps.sync_pull_interval_secs else {
        return Err(feature_refusal("sync_pull_interval_secs", NO_SYNC));
    };
    if inventory.marketplace().transport_class() != TransportClass::SellerDevice {
        return Err(validation(
            "this marketplace publishes an official API, so its catalogue is read on our own \
             infrastructure rather than on a cadence set here",
        ));
    }
    if !INTERVAL_CHOICES.contains(&body.interval_secs) {
        return Err(validation(
            "a sync interval is every six hours, every day, or every week",
        ));
    }
    if !body.publish_to.is_empty() && !caps.auto_publish_rules {
        return Err(feature_refusal("auto_publish_rules", NO_RULES));
    }
    if body.publish_to.contains(&inventory) {
        return Err(validation(
            "a resource pulled from a marketplace is already on it; a rule publishes to a \
             different one",
        ));
    }
    // A template the rule cannot honestly apply is refused rather than
    // silently ignored. A generic template fills catalogue fields and suits
    // any target; a scoped one is written for one marketplace's panel, and
    // naming it on a rule that publishes somewhere else would fill a
    // resource from a form the seller wrote for a different shop.
    if let Some(template) = body.template_id {
        if !caps.auto_publish_rules {
            return Err(feature_refusal("auto_publish_rules", NO_RULES));
        }
        let held = ResourceTemplateRepo::new(state.pool.clone())
            .get(context.org, template)
            .await
            .map_err(|error| storage_fault(&state, &error))?
            .ok_or_else(|| validation("that template is not one of yours"))?;
        if body.publish_to.is_empty() {
            return Err(validation(
                "a template fills what a rule publishes; add a marketplace to publish to first",
            ));
        }
        if let Some(scope) = held.scope.filter(|scope| !body.publish_to.contains(scope)) {
            return Err(validation(&format!(
                "\"{}\" is written for {}, which this shop does not publish to",
                held.name,
                name_of(scope),
            )));
        }
    }
    let now = (state.wall)();
    // Only switching it on needs the connection. Switching it off with a
    // connection since revoked has to stay possible, or a seller who
    // disconnected a shop could never stop it being asked for.
    if body.enabled {
        crate::import::source_connection(&state, context.org, inventory, now)
            .await
            .map_err(|_unused| {
                validation(
                    "Connect this marketplace on the Marketplaces page first, then switch sync \
                     on for it.",
                )
            })?;
    }
    // Floored rather than refused: the set above is what the control offers
    // and the floor is what this plan reaches, so a seller on a plan whose
    // shortest is a day and who asked for six hours gets a day rather than a
    // 422 about a number their own page had offered them.
    let interval_secs = body.interval_secs.max(minimum);
    let repo = SyncSettingRepo::new(state.pool.clone());
    repo.upsert(
        context.org,
        inventory,
        body.enabled,
        interval_secs,
        &body.publish_to,
        body.template_id,
    )
    .await
    .map_err(|error| storage_fault(&state, &error))?;
    let written = repo
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .into_iter()
        .find(|row| row.inventory == inventory)
        .ok_or_else(|| state.internal("the setting just written could not be read back"))?;
    Ok(Json(view_of(&written, Some(minimum))))
}

fn view_of(record: &SyncSettingRecord, minimum: Option<u32>) -> SyncSettingView {
    SyncSettingView {
        inventory: record.inventory,
        enabled: record.enabled,
        interval_secs: record.interval_secs,
        minimum_secs: minimum,
        last_pull_at: record.last_pull_at,
        publish_to: record.publish_to.clone(),
        template_id: record.template,
    }
}
