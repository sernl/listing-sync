//! Migrations: copying or moving catalogue resources between two
//! marketplaces.
//!
//! A migration is a `sync_request` whose resources are already canonicalised.
//! `POST /{version}/sync` addresses listings by the seller's own marketplace
//! locator and leaves the drain to resolve them, because a device-enumerated
//! import is the only thing that can turn an address on a shop we cannot read
//! into a product. A migration starts from the other end: the seller ticks
//! resources their catalogue already holds, and a resource is eligible
//! precisely because it carries a bound mapping on the source. The binding
//! names the listing, the catalogue names the product, and the removal leg's
//! breadcrumb is therefore complete before anything runs — so no marketplace
//! read stands between the confirm and the jobs.
//!
//! Two routes, and the first writes nothing. The preview is the whole point of
//! the design: a seller ticking forty resources is told, per resource, whether
//! it will be created, is already there, or is refused and why, against the
//! month's remaining allowance — before they spend any of it. The submit
//! re-runs the same plan rather than trusting the client's copy of it, so the
//! two can never disagree about what was admitted.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_domain::Mapping;
use tam_marketplace::{ListingState, RemoteListingId};
use tam_storage::{
    lower_head, uncaptured_source, CanonicalResource, Disposition, EntitlementRepo, HaltRepo,
    MappingRepo, NewMigration, ProductRepo, ProductSummary, SyncIntent, SyncRequestRepo,
};
use tam_types::{
    Currency, CurrencyRule, InventoryId, MappingId, Money, PriceIntent, ProductId, Timestamp, Uuid,
};

use crate::catalogue::unbound_mapping;
use crate::entitlement::{feature_refusal, migration_refusal, QuotaKind};
use crate::error::APIError;
use crate::jobs::{storage_fault, validation, RequestKey};
use crate::{AppState, OrgContext};

// -------------------------------------------------------------- vocabulary

/// What the preview says about one resource.
///
/// Three answers rather than two plus a message, because the console renders
/// them as three different chips and the seller's decision differs for each:
/// a blocked row is theirs to fix, an already-there row is nothing to do, and
/// only a will-create row spends the allowance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationVerdict {
    WillCreate,
    AlreadyThere,
    Blocked,
}

impl MigrationVerdict {
    /// The closed set, in a stable order, for the vocabulary generator.
    pub const ALL: [Self; 3] = [Self::WillCreate, Self::AlreadyThere, Self::Blocked];
}

// ------------------------------------------------------------------- wire

/// What the seller ticked.
///
/// Two spellings rather than one, for `SelectBody`'s reason: a tick list that
/// happens to name every resource is still a list, and a seller who asked for
/// all of them has named none.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MigrationSelection {
    All { all: bool },
    Products { products: Vec<ProductId> },
}

/// The one body both routes take, so the preview and the confirm cannot
/// describe two different migrations.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MigrationBody {
    pub source: InventoryId,
    pub target: InventoryId,
    /// `sync` copies and `migrate` moves; the wire spelling is
    /// `sync_request.disposition`'s own.
    pub disposition: String,
    pub selection: MigrationSelection,
}

/// Whether this pair of marketplaces can be migrated between at all, and why
/// not when it cannot.
///
/// Answered for every pair rather than only for the ones that work, because
/// the console offers every pair and disables the rest with the reason showing:
/// an option that is simply absent tells a seller nothing about when it might
/// appear.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairView {
    pub allowed: bool,
    pub reason: Option<String>,
}

/// The month's allowance measured against this selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapView {
    pub limit: u32,
    pub used: u32,
    pub remaining: u32,
    pub resets_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationRow {
    pub product: ProductId,
    pub title: String,
    pub verdict: MigrationVerdict,
    /// The listing this resource already has on the target, where it has one.
    pub remote: Option<String>,
    pub reason: Option<String>,
    /// What the seller should check after the move, where the two
    /// marketplaces price in different currencies and nothing converts.
    pub price_note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MigrationCounts {
    pub will_create: u32,
    pub already_there: u32,
    pub blocked: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationPlanView {
    pub pair: PairView,
    pub cap: CapView,
    pub rows: Vec<MigrationRow>,
    pub counts: MigrationCounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationAck {
    pub request: Uuid,
    pub queued: u32,
    pub skipped: u32,
}

// ------------------------------------------------------------------ handlers

/// The preview. Reads the catalogue, the two inventories' mappings, the halt
/// table and the month's usage, and writes nothing.
pub(crate) async fn plan_migration(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<MigrationBody>,
) -> Result<Json<MigrationPlanView>, APIError> {
    let plan = plan(&state, &context, &body).await?;
    Ok(Json(plan.view))
}

/// The confirm. Re-plans, admits the will-create rows, and writes the request
/// with every resource already canonicalised.
pub(crate) async fn create_migration(
    State(state): State<AppState>,
    context: OrgContext,
    key: RequestKey,
    Json(body): Json<MigrationBody>,
) -> Result<Response, APIError> {
    // The capability before the count. A plan that holds none of this
    // capability is not exhausted for the month, it does not include moving
    // resources at all, and the two refusals send the console to different
    // places.
    let caps = context.entitlement.caps;
    if caps.migrations_per_month == 0 {
        return Err(feature_refusal(
            "migrations_per_month",
            &QuotaKind::MigrationsPerMonth.sentence(0),
        ));
    }
    // A retried confirm is the same migration, answered before the plan is
    // recomputed: the first confirm queued the creates, so a fresh plan now
    // admits nothing and would refuse a request that already exists.
    let requests = SyncRequestRepo::new(state.pool.clone());
    if let Some(existing) = requests
        .get(context.org, key.0)
        .await
        .map_err(|error| storage_fault(&state, &error))?
    {
        let queued = u32::try_from(existing.resources.len()).unwrap_or(u32::MAX);
        return Ok((
            StatusCode::OK,
            Json(MigrationAck {
                request: key.0,
                queued,
                skipped: 0,
            }),
        )
            .into_response());
    }
    let plan = plan(&state, &context, &body).await?;
    if let Some(reason) = plan.view.pair.reason.filter(|_| !plan.view.pair.allowed) {
        return Err(validation(&reason));
    }
    let queued = plan.view.counts.will_create;
    let skipped = plan
        .view
        .counts
        .already_there
        .saturating_add(plan.view.counts.blocked);
    if queued == 0 {
        return Err(validation(
            "nothing in this selection can be created on that marketplace: every resource is \
             either already there or blocked, and the reasons are on each row",
        ));
    }
    // Counted in resources and re-checked here rather than trusted from the
    // preview, which is a read the seller may have left open for an hour while
    // another tab spent the same allowance.
    let used = i64::from(plan.view.cap.used);
    if used.saturating_add(i64::from(queued)) > i64::from(caps.migrations_per_month) {
        return Err(migration_refusal(
            used,
            caps.migrations_per_month,
            i64::from(queued),
            plan.view.cap.resets_at,
        ));
    }

    let mappings = MappingRepo::new(state.pool.clone());
    let now = (state.wall)();
    let mut resources = Vec::with_capacity(plan.admitted.len());
    for admitted in &plan.admitted {
        // The target mapping is what the create leg projects, and a resource
        // that is not on the target yet has none. Minting it is exactly what
        // `POST /{version}/products/{product}/mappings` writes when a seller
        // adds a marketplace by hand, so a request that then fails to write
        // leaves them the mapping they would have got by ticking it rather
        // than an artefact of a half-run migration.
        let mapping = if let Some(existing) = admitted.target_mapping {
            existing
        } else {
            let mapping = MappingId(Uuid(*uuid::Uuid::new_v4().as_bytes()));
            let minted = unbound_mapping(
                context.org,
                admitted.product,
                body.target,
                mapping,
                admitted.price,
            );
            mint(&state, &mappings, &minted, body.target, now).await?
        };
        resources.push(CanonicalResource {
            locator: admitted.locator.clone(),
            product: admitted.product,
            mapping,
            source: admitted.source.clone(),
            source_state: admitted.source_state,
        });
    }

    let written = requests
        .create_canonicalised(
            context.org,
            &NewMigration {
                // The idempotency key is the request's identity, so a retried
                // confirm is the same migration rather than a second one.
                id: key.0,
                source: body.source,
                target: body.target,
                disposition: plan.disposition,
                // A migration drafts. Where the listing ends up live is the
                // publish control's decision on the job endpoint, which is the
                // same rule `add_mapping` follows.
                intent: SyncIntent::Draft,
                requested_at: now,
                resources,
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    // Drained here, in the same handler, because nothing else will: the
    // device-read migration drains from the page that finishes canonicalising
    // it, and a catalogue migration is canonicalised in full by the write
    // above. The drain mints the create job on the target and, for a Move,
    // the removal job on the source gated on the target binding; both are
    // ordinary jobs the seller's device claims. Keyed on the request, so a
    // replayed confirm drains nothing twice.
    if written {
        let run = tam_import::ImportRun {
            pool: state.pool.clone(),
            org: context.org,
            source: body.source,
            target: Some(body.target),
            now,
        };
        tam_sync_worker::drain_request(&requests, &run, key.0)
            .await
            .map_err(|error| match error {
                tam_sync_worker::DrainError::Storage(error) => storage_fault(&state, &error),
                // Both are answers about the seller's own listings that the
                // plan admitted a moment ago; a change between the two is a
                // thing to say rather than a fault.
                tam_sync_worker::DrainError::Locator(why) => validation(&why),
                tam_sync_worker::DrainError::Lowering(refusal) => validation(&refusal.to_string()),
            })?;
    }
    let status = if written {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(MigrationAck {
            request: key.0,
            queued,
            skipped,
        }),
    )
        .into_response())
}

/// Writes the minted mapping, and answers the row that already exists when a
/// second confirm raced this one.
///
/// `mapping_one_per_inventory` is what decides: two submits under the same key
/// both found no mapping on the target, and the loser's insert would otherwise
/// turn the double-click this endpoint absorbs into a fault.
async fn mint(
    state: &AppState,
    mappings: &MappingRepo,
    minted: &Mapping,
    target: InventoryId,
    now: Timestamp,
) -> Result<MappingId, APIError> {
    match mappings.insert(minted.org, minted, 0, now).await {
        Ok(()) => Ok(minted.id),
        Err(error) => mappings
            .heads_for_products(minted.org, target, &[minted.product])
            .await
            .map_err(|error| storage_fault(state, &error))?
            .first()
            .map(|head| head.id)
            .ok_or_else(|| storage_fault(state, &error)),
    }
}

// --------------------------------------------------------------- the plan

/// One resource the plan admitted, and everything writing it needs.
struct Admitted {
    product: ProductId,
    price: PriceIntent,
    locator: String,
    source: RemoteListingId,
    source_state: Option<ListingState>,
    /// The mapping on the target, where the product already has one that is
    /// not bound. `None` is a product the confirm mints one for.
    target_mapping: Option<MappingId>,
}

struct Plan {
    view: MigrationPlanView,
    disposition: Disposition,
    admitted: Vec<Admitted>,
}

async fn plan(
    state: &AppState,
    context: &OrgContext,
    body: &MigrationBody,
) -> Result<Plan, APIError> {
    let disposition = match body.disposition.as_str() {
        "sync" => Disposition::Sync,
        "migrate" => Disposition::Migrate,
        _ => return Err(validation("disposition is \"sync\" or \"migrate\"")),
    };
    let now = (state.wall)();
    let caps = context.entitlement.caps;
    let usage = EntitlementRepo::new(state.pool.clone())
        .usage(context.org, now)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let used = u32::try_from(usage.migrations_this_month).unwrap_or(u32::MAX);
    let cap = CapView {
        limit: caps.migrations_per_month,
        used,
        remaining: caps.migrations_per_month.saturating_sub(used),
        resets_at: usage.migrations_reset_at,
    };
    let pair = pair_view(body.source, body.target);

    let products = ProductRepo::new(state.pool.clone());
    let chosen = chosen_products(&products, context.org, &body.selection)
        .await
        .map_err(|error| storage_fault(state, &error))?;

    // A pair nothing can cross blocks every row with the one reason, rather
    // than reading two inventories' mappings to answer a question the pair
    // has already settled.
    if !pair.allowed {
        let reason = pair.reason.clone();
        let rows: Vec<MigrationRow> = chosen
            .iter()
            .map(|product| blocked(product, reason.clone().unwrap_or_default()))
            .collect();
        let counts = MigrationCounts {
            blocked: u32::try_from(rows.len()).unwrap_or(u32::MAX),
            ..MigrationCounts::default()
        };
        return Ok(Plan {
            view: MigrationPlanView {
                pair,
                cap,
                rows,
                counts,
            },
            disposition,
            admitted: Vec::new(),
        });
    }

    let ids: Vec<ProductId> = chosen.iter().map(|product| product.id).collect();
    let mappings = MappingRepo::new(state.pool.clone());
    let on_source = mappings
        .heads_for_products(context.org, body.source, &ids)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let on_target = mappings
        .heads_for_products(context.org, body.target, &ids)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let with_payload = products
        .with_payload(context.org, &ids)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let halts = HaltRepo::new(state.pool.clone())
        .inventory_halts()
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let halted = halts
        .iter()
        .find(|halt| halt.inventory == body.source || halt.inventory == body.target)
        .map(|halt| format!("{} is paused: {}", name_of(halt.inventory), halt.reason));
    let target_ids: Vec<MappingId> = on_target.iter().map(|head| head.id).collect();
    let queued = mappings
        .with_open_items(context.org, &target_ids)
        .await
        .map_err(|error| storage_fault(state, &error))?;

    let mut rows = Vec::with_capacity(chosen.len());
    let mut admitted = Vec::new();
    let mut counts = MigrationCounts::default();
    for product in &chosen {
        let head = on_source.iter().find(|head| head.product == product.id);
        let Some((source, source_head)) =
            head.and_then(|head| head.remote.clone().map(|remote| (remote, head)))
        else {
            // Not listed on the source, so there is nothing to copy: a
            // migration moves a listing this tree can see, and a product
            // authored here but never published there has no listing.
            rows.push(blocked(
                product,
                format!("not listed on {}", name_of(body.source)),
            ));
            counts.blocked = counts.blocked.saturating_add(1);
            continue;
        };
        if !with_payload.contains(&product.id) {
            // Silent until now: `mapping_seeds` inner-joins the payload file,
            // so a product without one simply yields no item and the create
            // job carries a short list nobody is told about.
            rows.push(blocked(product, "no file".to_owned()));
            counts.blocked = counts.blocked.saturating_add(1);
            continue;
        }
        if let Some(reason) = halted.clone() {
            rows.push(blocked(product, reason));
            counts.blocked = counts.blocked.saturating_add(1);
            continue;
        }
        let target_head = on_target.iter().find(|head| head.product == product.id);
        if let Some(remote) = target_head.and_then(|head| head.remote.as_ref()) {
            rows.push(MigrationRow {
                product: product.id,
                title: product.title.0.clone(),
                verdict: MigrationVerdict::AlreadyThere,
                remote: Some(locator_of(remote)),
                reason: None,
                price_note: None,
            });
            counts.already_there = counts.already_there.saturating_add(1);
            continue;
        }
        if let Some(refusal) =
            target_head.and_then(|head| lower_head(ListingState::Draft, body.target, head).err())
        {
            rows.push(blocked(product, refusal.to_string()));
            counts.blocked = counts.blocked.saturating_add(1);
            continue;
        }
        // Confirmed already and not yet claimed by a device: the binding says
        // nothing about it, and offering it again would queue a second create
        // that settles refused once the first has bound.
        if target_head.is_some_and(|head| queued.contains(&head.id)) {
            rows.push(blocked(
                product,
                format!(
                    "already on its way to {}; it will show as there once your device has sent it",
                    name_of(body.target)
                ),
            ));
            counts.blocked = counts.blocked.saturating_add(1);
            continue;
        }
        // A move takes the source listing down from the state the tree
        // observed it in, and the drain refuses a resource whose lifecycle
        // nobody recorded rather than posting one it guessed. Refusing the
        // row here is what stops that refusal arriving as a whole failed
        // request after the seller confirmed.
        let source_state = listing_state_of(&source_head.lifecycle_state);
        if disposition == Disposition::Migrate && source_state.is_none() {
            rows.push(blocked(
                product,
                "this listing's state is unknown; verify it first".to_owned(),
            ));
            counts.blocked = counts.blocked.saturating_add(1);
            continue;
        }
        rows.push(MigrationRow {
            product: product.id,
            title: product.title.0.clone(),
            verdict: MigrationVerdict::WillCreate,
            remote: None,
            reason: None,
            price_note: price_note(product.price, body.source, body.target),
        });
        counts.will_create = counts.will_create.saturating_add(1);
        admitted.push(Admitted {
            product: product.id,
            price: product.price,
            locator: locator_of(&source),
            source,
            source_state,
            target_mapping: target_head.map(|head| head.id),
        });
    }

    Ok(Plan {
        view: MigrationPlanView {
            pair,
            cap,
            rows,
            counts,
        },
        disposition,
        admitted,
    })
}

/// The seller's tick list resolved against the catalogue.
///
/// A named product the catalogue does not hold is ignored rather than
/// refused, which is `ImportRunRepo::select`'s rule: a stale tab naming a
/// resource that has since been deleted moves the rest instead of refusing the
/// whole list.
async fn chosen_products(
    products: &ProductRepo,
    org: tam_types::OrgId,
    selection: &MigrationSelection,
) -> Result<Vec<ProductSummary>, tam_storage::StorageError> {
    let all = products.list(org).await?;
    Ok(match selection {
        MigrationSelection::All { .. } => all,
        MigrationSelection::Products { products } => all
            .into_iter()
            .filter(|summary| products.contains(&summary.id))
            .collect(),
    })
}

fn blocked(product: &ProductSummary, reason: String) -> MigrationRow {
    MigrationRow {
        product: product.id,
        title: product.title.0.clone(),
        verdict: MigrationVerdict::Blocked,
        remote: None,
        reason: Some(reason),
        price_note: None,
    }
}

/// Whether these two marketplaces can be migrated between, and why not.
///
/// Every reason here is a fact about what this tree has captured rather than a
/// policy: the source's download, the target's adapter. They are stated as
/// sentences a seller reads, because the console renders them under a disabled
/// option and has nowhere else to put an explanation.
fn pair_view(source: InventoryId, target: InventoryId) -> PairView {
    let reason = if source == target {
        Some("a migration's source and target are two different marketplaces".to_owned())
    } else if let Some(capability) = uncaptured_source(source) {
        Some(format!(
            "We cannot download files from {} yet ({capability}), so it cannot be a \
             migration's source.",
            name_of(source),
        ))
    } else if target == InventoryId::Etsy {
        Some("Etsy is not connected to Teachouse yet".to_owned())
    } else {
        None
    };
    PairView {
        allowed: reason.is_none(),
        reason,
    }
}

/// What a seller calls each marketplace, which is not what `Debug` calls it.
const fn name_of(inventory: InventoryId) -> &'static str {
    match inventory {
        InventoryId::Tes => "Tes",
        InventoryId::Tpt => "TPT",
        InventoryId::Etsy => "Etsy",
    }
}

/// How the source marketplace addresses a listing, rendered as the locator a
/// `sync_request_resource` row carries.
pub(crate) fn locator_of(remote: &RemoteListingId) -> String {
    match remote {
        RemoteListingId::Tes { url } => url.clone(),
        RemoteListingId::Tpt { product_id } => product_id.to_string(),
        RemoteListingId::Etsy { listing_id } => listing_id.to_string(),
    }
}

/// The stored lifecycle as the removal leg needs it. Anything else -- absent,
/// or one of the moderation states no write addresses -- is a state nobody
/// observed, which [`lower`] also refuses rather than guesses at.
///
/// [`lower`]: tam_storage::lower
fn listing_state_of(stored: &str) -> Option<ListingState> {
    match stored {
        "draft" => Some(ListingState::Draft),
        "live" => Some(ListingState::Live),
        _ => None,
    }
}

/// What happens to a price crossing two marketplaces that name different
/// currencies.
///
/// Nothing converts it: `resolve_price` denominates the number in the target's
/// own fixed currency and no FX step exists anywhere in the tree. That is a
/// deliberate answer rather than an oversight, so the row says so and the
/// seller checks it, instead of finding a four-pound worksheet priced at four
/// dollars and concluding the migration mangled it.
fn price_note(price: PriceIntent, source: InventoryId, target: InventoryId) -> Option<String> {
    let PriceIntent::Paid(money) = price else {
        return None;
    };
    let (CurrencyRule::Fixed(from), CurrencyRule::Fixed(to)) =
        (source.currency_rule(), target.currency_rule())
    else {
        return None;
    };
    if from == to {
        return None;
    }
    Some(format!(
        "{} becomes {} on {}; check it after it lands.",
        amount(money, from),
        amount(money, to),
        name_of(target),
    ))
}

/// The same number under two symbols, which is precisely what the note is
/// about.
///
/// `div_euclid` rather than `/`: the workspace denies the integer-division
/// operator, and the euclidean pair is the one whose remainder is never
/// negative, so the minor half needs no sign correction.
fn amount(money: Money, currency: Currency) -> String {
    let minor = money.minor_units();
    let symbol = match currency {
        Currency::Gbp => '£',
        Currency::Usd => '$',
    };
    let (whole, part) = (minor.div_euclid(100), minor.rem_euclid(100));
    format!("{symbol}{whole}.{part:02}")
}

#[cfg(test)]
mod tests {
    use super::{price_note, InventoryId, Money, PriceIntent};
    use tam_types::Currency;

    fn paid(minor: i64) -> PriceIntent {
        PriceIntent::Paid(Money::new(minor, Currency::Gbp).expect("a positive price"))
    }

    /// The number crosses unchanged and the row says so.
    ///
    /// Nothing in the tree converts a price: both write models mint the
    /// denomination from the target's own fixed rule, so a £4.50 worksheet
    /// arrives on TPT at $4.50. The seller finding that out from the preview
    /// rather than from their own shop is the whole point of the note.
    #[test]
    fn a_price_crossing_two_currencies_says_the_number_carries() {
        assert_eq!(
            price_note(paid(450), InventoryId::Tes, InventoryId::Tpt).as_deref(),
            Some("£4.50 becomes $4.50 on TPT; check it after it lands."),
        );
    }

    /// A free resource has no number to carry, and a pair that prices in one
    /// currency has nothing to warn about. Both would be noise on every row.
    #[test]
    fn there_is_nothing_to_say_about_a_free_resource_or_a_single_currency() {
        assert_eq!(
            price_note(PriceIntent::Free, InventoryId::Tes, InventoryId::Tpt),
            None,
        );
        assert_eq!(
            price_note(paid(450), InventoryId::Tes, InventoryId::Tes),
            None,
        );
    }
}
