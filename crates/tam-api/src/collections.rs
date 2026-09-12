//! Collections over the wire: a named, ordered set of resources, and the four
//! things a seller does with one.
//!
//! The set is the point. A seller who has assembled "the autumn unit" wants to
//! publish it to a marketplace, finish its forms from a template, label it and
//! put it in a spreadsheet — and every one of those verbs already exists for a
//! tick list. So this module adds no new machinery for them: the publish is
//! `POST /{version}/jobs`' own lowering and minting, the template apply is the
//! apply route taking a collection instead of a list, the labels are
//! `LabelRepo::set_for_product` in a loop under the plan's own allowance, and
//! the export is the catalogue CSV with a filter. What a collection adds is
//! that the seller states the set once and it stays stated.
//!
//! Delisting a collection is deliberately absent. Taking forty listings down
//! is the one bulk verb whose mistake cannot be undone from here, and the
//! design does not ask for it.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_marketplace::ListingState;
use tam_storage::NewJobItem;
use tam_storage::{
    lower_head, CollectionChange, CollectionEdit, CollectionRecord, CollectionSummary, Given,
    HaltRepo, JobReadRepo, LabelRepo, MappingRepo, NewCollection, ProductRepo,
    ResourceCollectionRepo, COLLECTIONS_PER_ORG_MAX,
};
use tam_types::{Actor, InventoryId, JobId, MappingId, PriceIntent, ProductId, Timestamp, Uuid};

use crate::catalogue::unbound_mapping;
use crate::entitlement::{quota_refusal, QuotaKind};
use crate::error::APIError;
use crate::jobs::{mint_job, new_items, storage_fault, validation, RequestKey};
use crate::migrations::{locator_of, name_of, MigrationCounts, MigrationVerdict};
use crate::text::is_typed_text;
use crate::{AppState, OrgContext};

/// The longest collection name the form accepts, counted in characters rather
/// than bytes so a name written in accented or non-Latin letters is measured
/// the way the seller who typed it sees it. Migration 0072's own CHECK.
pub const NAME_MAX_CHARS: usize = 80;

/// And the longest note, matching the template's.
pub const DESCRIPTION_MAX_CHARS: usize = 1_000;

/// How many resources one `PUT .../members` may name.
///
/// The membership write is a statement per resource inside one transaction, so
/// the bound is on the transaction rather than on the table: a request naming
/// a hundred thousand resources would hold every row of the collection locked
/// while it ran. Far above a seller assembling a unit by hand and far below a
/// request that takes a visible amount of time.
pub const MEMBERS_MAX: usize = 500;

// ------------------------------------------------------------------- wire

/// One collection as a list row renders it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionHead {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    /// How many resources it holds, not counting any in the bin.
    pub count: i64,
    /// The marketplaces its members are listed on, which is the mark the list
    /// renders. Answered here rather than left to a read per row, because a
    /// list of thirty collections would otherwise be thirty-one requests.
    pub inventories: Vec<InventoryId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl CollectionHead {
    fn of(summary: CollectionSummary) -> Self {
        Self {
            id: summary.id,
            name: summary.name,
            description: summary.description,
            count: summary.count,
            inventories: summary.inventories,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionsView {
    pub collections: Vec<CollectionHead>,
}

/// One member, in the seller's own order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionMemberView {
    pub product: ProductId,
    pub title: String,
    pub position: i32,
    pub inventories: Vec<InventoryId>,
}

/// One collection whole, which is what its page reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionView {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub count: i64,
    pub members: Vec<CollectionMemberView>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateCollectionBody {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// What an edit replaces. Both absent is refused, because the write would
/// still move `updated_at` and misstate when the seller last touched it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateCollectionBody {
    #[serde(default)]
    pub name: Option<String>,
    /// [`Given`] rather than an option, so an absent note keeps the stored
    /// one and an explicit null clears it.
    #[serde(default, deserialize_with = "crate::resource_templates::given")]
    pub description: Given<String>,
}

/// The whole ordered membership. A list rather than a list of pairs: the
/// position is the index, so an order the client states twice is an order two
/// clients can disagree about.
#[derive(Debug, Clone, Deserialize)]
pub struct MembersBody {
    pub products: Vec<ProductId>,
}

/// The labels to add to every member.
///
/// Add rather than replace, unlike one resource's own label route: a seller
/// labelling a collection is marking a set, and replacing forty resources'
/// labels with one word would delete work that has nothing to do with this
/// collection.
#[derive(Debug, Clone, Deserialize)]
pub struct AddLabelsBody {
    pub add: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedLabelsView {
    /// The labels as stored, trimmed the way the label route trims them.
    pub added: Vec<String>,
    /// How many resources carry them now.
    pub members: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PublishBody {
    pub inventory: InventoryId,
    /// `draft` or `live`, the same two words `POST /{version}/jobs` takes.
    #[serde(default)]
    pub intent: Option<String>,
}

/// What the preview says about one member. The verdict vocabulary is the
/// migration preview's, because the question is the same one — will this
/// resource be created on that marketplace, is it already there, or is it
/// refused and why — and a second three-word enum would be a second thing for
/// the console to render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionPublishRow {
    pub product: ProductId,
    pub title: String,
    pub verdict: MigrationVerdict,
    /// The listing this resource already has there, where it has one.
    pub remote: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionPublishPlanView {
    pub inventory: InventoryId,
    pub intent: String,
    pub rows: Vec<CollectionPublishRow>,
    pub counts: MigrationCounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionPublishAck {
    /// The job the admitted members were queued on, or `null` where nothing
    /// was admitted and no job was minted.
    pub job: Option<JobId>,
    pub queued: u32,
    pub skipped: u32,
}

// ------------------------------------------------------------- validation

fn parse_id(raw: &str) -> Result<Uuid, APIError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(|parsed| Uuid(*parsed.as_bytes()))
        .map_err(|_unused| validation("that is not a collection identifier"))
}

fn missing() -> APIError {
    crate::error::APIError::new(
        StatusCode::NOT_FOUND,
        crate::error::APIErrorEntry::new("no such collection")
            .kind(crate::error::APIErrorKind::NotFound),
    )
}

/// The name as it will be stored, or the refusal a seller can act on.
///
/// Trimmed rather than refused for surrounding space, and measured after
/// trimming, so a name is bounded as it will be stored. Control characters are
/// refused here rather than by Postgres, because no `text` column can hold a
/// zero byte and one arriving at the column is a fault where an answer
/// belongs.
fn validated_name(raw: &str) -> Result<&str, APIError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(validation("a collection needs a name"));
    }
    if trimmed.chars().count() > NAME_MAX_CHARS {
        return Err(validation(&format!(
            "a collection name is at most {NAME_MAX_CHARS} characters"
        )));
    }
    if !is_typed_text(trimmed) {
        return Err(validation(
            "a collection name cannot contain control characters",
        ));
    }
    Ok(trimmed)
}

/// The note as it will be stored, and `None` where the seller wrote nothing:
/// two spellings of "no note" would make every reader check for each, which is
/// what migration 0072's CHECK refuses.
fn validated_description(raw: Option<&str>) -> Result<Option<&str>, APIError> {
    let Some(trimmed) = raw.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if trimmed.chars().count() > DESCRIPTION_MAX_CHARS {
        return Err(validation(&format!(
            "a collection note is at most {DESCRIPTION_MAX_CHARS} characters"
        )));
    }
    if trimmed
        .chars()
        .any(|character| character.is_control() && character != '\n' && character != '\r')
    {
        return Err(validation(
            "a collection note cannot contain control characters",
        ));
    }
    Ok(Some(trimmed))
}

// ------------------------------------------------------------------ handlers

/// Every collection this organisation has, with each one's live member count
/// and marketplace marks.
///
/// Unpaged, because [`COLLECTIONS_PER_ORG_MAX`] bounds it and a picker that
/// arrives in pages is not a picker.
pub(crate) async fn list(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<CollectionsView>, APIError> {
    let held = ResourceCollectionRepo::new(state.pool.clone())
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(CollectionsView {
        collections: held.into_iter().map(CollectionHead::of).collect(),
    }))
}

/// Which collections hold one resource, which is what the resource page's own
/// panel reads.
///
/// Filtered from the same listing rather than answered by a second aggregate:
/// the listing is unpaged and bounded, so this is two reads for the whole
/// panel instead of one per collection the resource is in.
pub(crate) async fn for_product(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, product)): Path<(String, String)>,
) -> Result<Json<CollectionsView>, APIError> {
    let product = ProductId(
        parse_id(&product).map_err(|_unused| validation("that is not a resource identifier"))?,
    );
    let repo = ResourceCollectionRepo::new(state.pool.clone());
    let holding: Vec<Uuid> = repo
        .for_product(context.org, product)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .into_iter()
        .map(|record| record.id)
        .collect();
    let collections = repo
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .into_iter()
        .filter(|summary| holding.contains(&summary.id))
        .map(CollectionHead::of)
        .collect();
    Ok(Json(CollectionsView { collections }))
}

/// One collection whole, with its members in the seller's own order.
///
/// A collection of another organisation answers not-found rather than
/// forbidden, because the identifier is the one thing a caller could guess and
/// the two answers would tell them which guesses were right.
pub(crate) async fn get(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, collection)): Path<(String, String)>,
) -> Result<Json<CollectionView>, APIError> {
    let id = parse_id(&collection)?;
    let repo = ResourceCollectionRepo::new(state.pool.clone());
    let record = repo
        .get(context.org, id)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(missing)?;
    view_of(&state, &context, &record).await
}

pub(crate) async fn create(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<CreateCollectionBody>,
) -> Result<(StatusCode, Json<CollectionView>), APIError> {
    let name = validated_name(&body.name)?;
    let description = validated_description(body.description.as_deref())?;
    let repo = ResourceCollectionRepo::new(state.pool.clone());
    // The plan's allowance before the store's absolute ceiling: one is what
    // this seller bought and the other is what the unpaged listing can render,
    // and a seller on Free meeting "two hundred collections" would be told
    // about a bound that is not theirs.
    let held = repo
        .count(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if held >= i64::from(context.entitlement.caps.collections_max) {
        return Err(quota_refusal(
            QuotaKind::Collections,
            held,
            u64::from(context.entitlement.caps.collections_max),
        ));
    }
    let written = repo
        .create(
            context.org,
            &NewCollection {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                name,
                description,
                created_at: (state.wall)(),
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match written {
        tam_storage::CollectionWrite::Saved(record) => {
            let view = view_of(&state, &context, &record).await?;
            Ok((StatusCode::CREATED, view))
        }
        // Validation rather than a conflict status, for the reason every other
        // refusal on this surface is: the client renders the sentence.
        tam_storage::CollectionWrite::NameTaken => {
            Err(validation("you already have a collection of that name"))
        }
        tam_storage::CollectionWrite::TooMany => Err(validation(&format!(
            "you have {COLLECTIONS_PER_ORG_MAX} collections already; \
             remove one before making another"
        ))),
    }
}

/// Renames a collection, replaces its note, or both. An absent field keeps its
/// value and an explicit null clears the note.
pub(crate) async fn update(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, collection)): Path<(String, String)>,
    Json(body): Json<UpdateCollectionBody>,
) -> Result<Json<CollectionView>, APIError> {
    let id = parse_id(&collection)?;
    let name = match body.name.as_deref() {
        Some(raw) => Some(validated_name(raw)?),
        None => None,
    };
    let description = match &body.description {
        Given::Kept => Given::Kept,
        Given::Set(note) => Given::Set(validated_description(note.as_deref())?),
    };
    let edit = CollectionEdit::of(name, description)
        .ok_or_else(|| validation("an edit names a new name, a new note, or both"))?;
    let written = ResourceCollectionRepo::new(state.pool.clone())
        .update(context.org, id, &edit, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match written {
        CollectionChange::Saved(record) => view_of(&state, &context, &record).await,
        CollectionChange::Missing => Err(missing()),
        CollectionChange::NameTaken => {
            Err(validation("you already have a collection of that name"))
        }
    }
}

/// Removes one collection. No resource is touched: a collection is a way of
/// looking at the catalogue and not part of it.
pub(crate) async fn delete(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, collection)): Path<(String, String)>,
) -> Result<StatusCode, APIError> {
    let id = parse_id(&collection)?;
    let removed = ResourceCollectionRepo::new(state.pool.clone())
        .delete(context.org, id)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(missing())
    }
}

/// Replaces the whole membership, in the order given.
///
/// A named resource the catalogue does not hold is dropped rather than
/// refusing the request, which is the rule every selection on this surface
/// follows: a stale tab naming a resource that has since been deleted files
/// the rest.
pub(crate) async fn set_members(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, collection)): Path<(String, String)>,
    Json(body): Json<MembersBody>,
) -> Result<Json<CollectionView>, APIError> {
    let id = parse_id(&collection)?;
    if body.products.len() > MEMBERS_MAX {
        return Err(validation(&format!(
            "a collection holds at most {MEMBERS_MAX} resources"
        )));
    }
    let held = ProductRepo::new(state.pool.clone())
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    // The seller's order, kept: the filter walks their list rather than the
    // catalogue's, because the catalogue's order is by creation and theirs is
    // the one the position records.
    let mut members: Vec<ProductId> = Vec::with_capacity(body.products.len());
    for product in &body.products {
        if held.iter().any(|summary| summary.id == *product) && !members.contains(product) {
            members.push(*product);
        }
    }
    let repo = ResourceCollectionRepo::new(state.pool.clone());
    if !repo
        .set_members(context.org, id, &members, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?
    {
        return Err(missing());
    }
    let record = repo
        .get(context.org, id)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(missing)?;
    view_of(&state, &context, &record).await
}

/// Adds labels to every member, under the plan's own vocabulary allowance.
///
/// Server-side rather than a loop in the browser, which is what the single
/// resource's label dialog does: forty round trips from a tab the seller may
/// close halfway is a half-labelled collection nobody can tell from a
/// finished one. The allowance is checked once, against the labels this
/// request would mint, because minting is what spends it — a label the
/// organisation already has costs nothing however many resources carry it.
///
/// A system label is refused rather than quietly dropped, as the resource's
/// own route refuses it: its name is ours, the console never sends it, and a
/// client that does is asking for a label whose colour and flag this route
/// would not write.
pub(crate) async fn add_labels(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, collection)): Path<(String, String)>,
    Json(body): Json<AddLabelsBody>,
) -> Result<Json<AddedLabelsView>, APIError> {
    let id = parse_id(&collection)?;
    let names: Vec<String> = body
        .add
        .iter()
        .map(|name| tam_storage::labels::normalise(name))
        .filter(|name| !name.is_empty())
        .collect();
    if names.is_empty() {
        return Err(validation("name at least one label to add"));
    }
    let labels = LabelRepo::new(state.pool.clone());
    let every = labels
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if let Some(claimed) = names.iter().find(|name| {
        every
            .iter()
            .any(|record| record.system && record.name.eq_ignore_ascii_case(name))
    }) {
        return Err(validation(&format!(
            "{claimed} is the label of a marketplace you imported from; it is set for you and \
             cannot be typed or removed"
        )));
    }
    let own: Vec<&tam_storage::LabelRecord> =
        every.iter().filter(|record| !record.system).collect();
    let fresh = names
        .iter()
        .filter(|name| {
            !own.iter()
                .any(|record| record.name.eq_ignore_ascii_case(name))
        })
        .count();
    let allowed = usize::try_from(context.entitlement.caps.labels_max).unwrap_or(usize::MAX);
    if fresh > 0 && own.len().saturating_add(fresh) > allowed {
        return Err(quota_refusal(
            QuotaKind::Labels,
            i64::try_from(own.len()).unwrap_or(i64::MAX),
            u64::from(context.entitlement.caps.labels_max),
        ));
    }
    let members = ResourceCollectionRepo::new(state.pool.clone())
        .members(context.org, id)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let now = (state.wall)();
    let mut touched = 0u32;
    for member in &members {
        // Read then write, because the repository's own write replaces the
        // seller's whole set for one resource: the union is what "add" means,
        // and the system labels it leaves alone are left alone by that write.
        let carried = labels
            .for_product(context.org, member.product)
            .await
            .map_err(|error| storage_fault(&state, &error))?;
        let mut wanted: Vec<String> = carried
            .iter()
            .filter(|record| !record.system)
            .map(|record| record.name.clone())
            .collect();
        for name in &names {
            if !wanted.iter().any(|held| held.eq_ignore_ascii_case(name)) {
                wanted.push(name.clone());
            }
        }
        labels
            .set_for_product(context.org, member.product, &wanted, now)
            .await
            .map_err(|error| storage_fault(&state, &error))?;
        touched = touched.saturating_add(1);
    }
    Ok(Json(AddedLabelsView {
        added: names,
        members: touched,
    }))
}

// -------------------------------------------------------------- publishing

/// The preview. Reads the membership, the target's mappings, the halt table
/// and the open items, and writes nothing.
pub(crate) async fn plan_publish(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, collection)): Path<(String, String)>,
    Json(body): Json<PublishBody>,
) -> Result<Json<CollectionPublishPlanView>, APIError> {
    let plan = publish_plan(&state, &context, &collection, &body).await?;
    Ok(Json(plan.view))
}

/// The confirm. Re-plans, mints a mapping for each admitted member that has
/// none, and queues them all on one job.
///
/// One job rather than one per resource, because a job is a unit of work the
/// seller's device claims and forty of them for one button is forty leases,
/// forty ledgers and forty rows on the jobs page for one intention. The key is
/// the caller's `Idempotency-Key`, so a double-click replays the first job
/// rather than minting a second — the same contract `POST /{version}/jobs`
/// has.
pub(crate) async fn publish(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, collection)): Path<(String, String)>,
    key: RequestKey,
    Json(body): Json<PublishBody>,
) -> Result<Json<CollectionPublishAck>, APIError> {
    let plan = publish_plan(&state, &context, &collection, &body).await?;
    let skipped = plan
        .view
        .counts
        .already_there
        .saturating_add(plan.view.counts.blocked);
    if plan.admitted.is_empty() {
        return Ok(Json(CollectionPublishAck {
            job: None,
            queued: 0,
            skipped,
        }));
    }
    let now = (state.wall)();
    let mappings = MappingRepo::new(state.pool.clone());
    let mut ids = Vec::with_capacity(plan.admitted.len());
    for admitted in &plan.admitted {
        let mapping = if let Some(existing) = admitted.mapping {
            existing
        } else {
            let id = MappingId(Uuid(*uuid::Uuid::new_v4().as_bytes()));
            let minted = unbound_mapping(
                context.org,
                admitted.product,
                body.inventory,
                id,
                admitted.price,
            );
            crate::migrations::mint(&state, &mappings, &minted, body.inventory, now).await?
        };
        ids.push(mapping);
    }
    let seeds = JobReadRepo::new(state.pool.clone())
        .mapping_seeds(context.org, body.inventory, &ids)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let job = JobId(Uuid(*uuid::Uuid::new_v4().as_bytes()));
    let to = intent_state(&plan.view.intent);
    let mut items: Vec<NewJobItem> = Vec::new();
    for seed in &seeds {
        let operations = tam_storage::lower(to, body.inventory, seed)
            .map_err(|refusal| validation(&refusal.to_string()))?;
        items.extend(new_items(
            context.org,
            body.inventory,
            job,
            seed,
            operations,
        ));
    }
    if items.is_empty() {
        return Ok(Json(CollectionPublishAck {
            job: None,
            queued: 0,
            skipped,
        }));
    }
    let created = mint_job(
        &state,
        context.org,
        job,
        body.inventory,
        Actor::Person(context.user),
        now,
        key.0,
        &items,
    )
    .await?;
    Ok(Json(CollectionPublishAck {
        job: Some(created.job),
        queued: plan.view.counts.will_create,
        skipped,
    }))
}

struct PublishAdmitted {
    product: ProductId,
    price: PriceIntent,
    /// The mapping on the target, where the resource already has an unbound
    /// one. `None` is a resource the confirm mints one for.
    mapping: Option<MappingId>,
}

struct PublishPlan {
    view: CollectionPublishPlanView,
    admitted: Vec<PublishAdmitted>,
}

const fn intent_state(intent: &str) -> ListingState {
    if matches!(intent.as_bytes(), b"live") {
        ListingState::Live
    } else {
        ListingState::Draft
    }
}

async fn publish_plan(
    state: &AppState,
    context: &OrgContext,
    collection: &str,
    body: &PublishBody,
) -> Result<PublishPlan, APIError> {
    let id = parse_id(collection)?;
    let intent = match body.intent.as_deref() {
        None | Some("draft") => "draft".to_owned(),
        Some("live") => "live".to_owned(),
        Some(_) => return Err(validation("intent is \"draft\" or \"live\"")),
    };
    let members = ResourceCollectionRepo::new(state.pool.clone())
        .members(context.org, id)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let products: Vec<ProductId> = members.iter().map(|member| member.product).collect();
    let prices = ProductRepo::new(state.pool.clone());
    let catalogue = prices
        .list(context.org)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    // What a listing that does not exist yet would need, for every member at
    // once. One read rather than one per member: a collection is forty
    // resources and this is asked of each of them.
    let facts = prices
        .creation_facts(context.org, &products, body.inventory)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let mappings = MappingRepo::new(state.pool.clone());
    let on_target = mappings
        .heads_for_products(context.org, body.inventory, &products)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let queued = mappings
        .with_open_items(
            context.org,
            &on_target.iter().map(|head| head.id).collect::<Vec<_>>(),
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let halted = HaltRepo::new(state.pool.clone())
        .inventory_halts()
        .await
        .map_err(|error| storage_fault(state, &error))?
        .into_iter()
        .find(|halt| halt.inventory == body.inventory)
        .map(|halt| format!("{} is paused: {}", name_of(halt.inventory), halt.reason));

    let to = intent_state(&intent);
    let mut rows = Vec::with_capacity(members.len());
    let mut admitted = Vec::new();
    let mut counts = MigrationCounts::default();
    for member in &members {
        let row = |verdict, remote, reason| CollectionPublishRow {
            product: member.product,
            title: member.title.clone(),
            verdict,
            remote,
            reason,
        };
        let head = on_target.iter().find(|head| head.product == member.product);
        if let Some(remote) = head.and_then(|head| head.remote.as_ref()) {
            rows.push(row(
                MigrationVerdict::AlreadyThere,
                Some(locator_of(remote)),
                None,
            ));
            counts.already_there = counts.already_there.saturating_add(1);
            continue;
        }
        // Everything a listing that does not exist yet needs, asked after the
        // already-there arm above: a member the target already carries is
        // having nothing created for it. Silent until now — `mapping_seeds`
        // inner-joins the payload file, so a member without one yielded no
        // item and the job carried a short list nobody was told about — and
        // the target's own required fields were never asked at all.
        if let Some(why) = facts
            .iter()
            .find(|facts| facts.product == member.product)
            .and_then(|facts| crate::catalogue::creation_blocked(facts, body.inventory))
        {
            rows.push(row(MigrationVerdict::Blocked, None, Some(why.reason())));
            counts.blocked = counts.blocked.saturating_add(1);
            continue;
        }
        if let Some(reason) = halted.clone() {
            rows.push(row(MigrationVerdict::Blocked, None, Some(reason)));
            counts.blocked = counts.blocked.saturating_add(1);
            continue;
        }
        if head.is_some_and(|head| queued.contains(&head.id)) {
            rows.push(row(
                MigrationVerdict::Blocked,
                None,
                Some(format!(
                    "already on its way to {}; it will show as there once your device has sent it",
                    name_of(body.inventory)
                )),
            ));
            counts.blocked = counts.blocked.saturating_add(1);
            continue;
        }
        if let Some(refusal) = head.and_then(|head| lower_head(to, body.inventory, head).err()) {
            rows.push(row(
                MigrationVerdict::Blocked,
                None,
                Some(refusal.to_string()),
            ));
            counts.blocked = counts.blocked.saturating_add(1);
            continue;
        }
        rows.push(row(MigrationVerdict::WillCreate, None, None));
        counts.will_create = counts.will_create.saturating_add(1);
        admitted.push(PublishAdmitted {
            product: member.product,
            price: catalogue
                .iter()
                .find(|summary| summary.id == member.product)
                .map_or(PriceIntent::Free, |summary| summary.price),
            mapping: head.map(|head| head.id),
        });
    }

    Ok(PublishPlan {
        view: CollectionPublishPlanView {
            inventory: body.inventory,
            intent,
            rows,
            counts,
        },
        admitted,
    })
}

// -------------------------------------------------------------------- views

async fn view_of(
    state: &AppState,
    context: &OrgContext,
    record: &CollectionRecord,
) -> Result<Json<CollectionView>, APIError> {
    let members = ResourceCollectionRepo::new(state.pool.clone())
        .members(context.org, record.id)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(Json(CollectionView {
        id: record.id,
        name: record.name.clone(),
        description: record.description.clone(),
        count: i64::try_from(members.len()).unwrap_or(i64::MAX),
        members: members
            .into_iter()
            .map(|member| CollectionMemberView {
                product: member.product,
                title: member.title,
                position: member.position,
                inventories: member.inventories,
            })
            .collect(),
        created_at: record.created_at,
        updated_at: record.updated_at,
    }))
}
