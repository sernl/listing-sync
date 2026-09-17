//! Seller-owned target proposals. Accepting one never edits the source catalogue.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use tam_domain::seller_rules::{
    self as rules, RuleAction, RuleKind, RuleMatch, RuleOverrides, RulePreset,
    SellerRuleDefinition, SellerRuleRecord, TargetFields,
};
use tam_storage::{
    seller_rules::{
        AcceptedChoice, DecisionKind, DecisionOutcome, NewPreview, NewPreviewRow,
        NewReferenceQuote, PreviewDecision, PreviewStatus, RuleDecision, RuleDelete, RuleEdit,
        RuleQuery, RuleScope, RuleState, RuleWriteRefusal, Selection, SellerRuleRepo, StaleReason,
    },
    ProductRepo,
};
use tam_types::{Currency, InventoryId, PriceIntent, ProductId, Timestamp, Uuid};

use crate::exchange_rates::{ECB_DATA_URL, ECB_NOTICE};
use crate::{APIError, APIErrorEntry, APIErrorKind, AppState, OrgContext};

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

fn missing() -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new("The requested rule or proposal is unavailable.")
            .kind(APIErrorKind::NotFound),
    )
}

fn conflict(message: &str) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

fn parse_id(raw: &str) -> Result<uuid::Uuid, APIError> {
    uuid::Uuid::parse_str(raw).map_err(|_| validation("The identifier is not a UUID."))
}

fn fresh_id() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}

fn first_page() -> u32 {
    1
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateFilter {
    #[default]
    All,
    Enabled,
    Disabled,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub kind: Option<RuleKind>,
    #[serde(default)]
    pub state: StateFilter,
    pub source: Option<InventoryId>,
    pub target: Option<InventoryId>,
    pub q: Option<String>,
    #[serde(default = "first_page")]
    pub page: u32,
}

#[derive(Debug, Serialize)]
pub struct Counts {
    pub all: i64,
    pub enabled: i64,
    pub disabled: i64,
}

#[derive(Debug, Serialize)]
pub struct RuleList {
    pub rows: Vec<SellerRuleRecord>,
    pub page: u32,
    pub has_next: bool,
    pub counts: Counts,
}

pub(crate) async fn list(
    State(state): State<AppState>,
    context: OrgContext,
    Query(query): Query<ListQuery>,
) -> Result<Json<RuleList>, APIError> {
    if query.page == 0 {
        return Err(validation("Page numbers start at one."));
    }
    let page = SellerRuleRepo::new(state.pool.clone())
        .list(
            context.org,
            &RuleQuery {
                kind: query.kind,
                state: match query.state {
                    StateFilter::All => RuleState::All,
                    StateFilter::Enabled => RuleState::Enabled,
                    StateFilter::Disabled => RuleState::Disabled,
                },
                source: query.source,
                target: query.target,
                search: query.q,
                page: query.page,
                page_size: 25,
            },
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(RuleList {
        rows: page.rows,
        page: page.page,
        has_next: page.has_next,
        counts: Counts {
            all: page.counts.all,
            enabled: page.counts.enabled,
            disabled: page.counts.disabled,
        },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRule {
    pub definition: SellerRuleDefinition,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRule {
    pub revision: i64,
    pub definition: SellerRuleDefinition,
}

async fn validate_rule(
    state: &AppState,
    context: &OrgContext,
    definition: &SellerRuleDefinition,
) -> Result<(), APIError> {
    rules::validate_definition(definition).map_err(|error| validation(&error.to_string()))?;
    if let RuleAction::Pricing {
        rate,
        reference: Some(id),
        ..
    } = &definition.action
    {
        let quote = SellerRuleRepo::new(state.pool.clone())
            .reference(context.org, uuid::Uuid::from_bytes(id.0))
            .await
            .map_err(|error| state.internal(&error.to_string()))?
            .ok_or_else(|| {
                validation("The reference quote is unavailable. Refresh it or enter a manual rate.")
            })?;
        let expected = rules::parse_rate(rate).map_err(|error| validation(&error.to_string()))?;
        if quote.rate_micros != expected
            || definition.source.currency_rule() != tam_types::CurrencyRule::Fixed(quote.source)
            || definition.target.currency_rule() != tam_types::CurrencyRule::Fixed(quote.target)
        {
            return Err(validation("The reference quote does not match this rate and currency direction. Clear the reference when entering your own rate."));
        }
    }
    Ok(())
}

pub(crate) async fn create(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<CreateRule>,
) -> Result<(StatusCode, Json<SellerRuleRecord>), APIError> {
    validate_rule(&state, &context, &body.definition).await?;
    let saved = SellerRuleRepo::new(state.pool.clone())
        .create(context.org, context.user, &body.definition, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok((StatusCode::CREATED, Json(saved)))
}

pub(crate) async fn update(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, raw)): Path<(String, String)>,
    Json(body): Json<UpdateRule>,
) -> Result<Json<SellerRuleRecord>, APIError> {
    let id = parse_id(&raw)?;
    validate_rule(&state, &context, &body.definition).await?;
    match SellerRuleRepo::new(state.pool.clone())
        .update(
            context.org,
            &RuleEdit {
                id,
                revision: body.revision,
                definition: &body.definition,
                author: context.user,
                at: (state.wall)(),
            },
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))?
    {
        Ok(record) => Ok(Json(record)),
        Err(RuleWriteRefusal::Stale { .. }) => Err(conflict(
            "This rule changed. Reload it before saving your edit.",
        )),
        Err(RuleWriteRefusal::Missing) => Err(missing()),
    }
}

#[derive(Debug, Deserialize)]
pub struct Revision {
    pub revision: i64,
}

pub(crate) async fn delete(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, raw)): Path<(String, String)>,
    Query(query): Query<Revision>,
) -> Result<StatusCode, APIError> {
    match SellerRuleRepo::new(state.pool.clone())
        .delete(context.org, parse_id(&raw)?, query.revision)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
    {
        RuleDelete::Deleted => Ok(StatusCode::NO_CONTENT),
        RuleDelete::Stale { .. } => {
            Err(conflict("This rule changed. Reload it before deleting it."))
        }
        RuleDelete::Missing => Err(missing()),
    }
}

#[derive(Debug, Serialize)]
pub struct Presets {
    pub presets: Vec<RulePreset>,
}

pub(crate) fn presets(_context: OrgContext) -> std::future::Ready<Json<Presets>> {
    std::future::ready(Json(Presets {
        presets: rules::presets(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ReferenceQuery {
    pub source: Currency,
    pub target: Currency,
}

#[derive(Debug, Serialize)]
pub struct ReferenceView {
    pub id: Uuid,
    pub source: Currency,
    pub target: Currency,
    pub rate: String,
    pub rate_micros: i64,
    pub as_of: String,
    pub provider: String,
    pub source_url: String,
    pub fetched_at: Timestamp,
    pub notice: &'static str,
}

fn rate_string(micros: i64) -> String {
    let whole = micros.div_euclid(1_000_000);
    let fraction = micros.rem_euclid(1_000_000);
    if fraction == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{fraction:06}")
            .trim_end_matches('0')
            .to_owned()
    }
}

pub(crate) async fn reference(
    State(state): State<AppState>,
    context: OrgContext,
    Query(query): Query<ReferenceQuery>,
) -> Result<Json<ReferenceView>, APIError> {
    if query.source == query.target {
        return Err(validation("Choose different source and target currencies."));
    }
    let unavailable = || {
        APIError::new(StatusCode::SERVICE_UNAVAILABLE, APIErrorEntry::new("The ECB reference is unavailable. Your accepted prices are unchanged; retry or enter a manual estimate.").kind(APIErrorKind::Internal))
    };
    let source = state.exchange_rates.as_ref().ok_or_else(unavailable)?;
    let observation = source
        .fetch(query.source, query.target)
        .await
        .map_err(|_| unavailable())?;
    if observation.source != query.source
        || observation.target != query.target
        || observation.rate_micros <= 0
    {
        return Err(unavailable());
    }
    let as_of = sqlx::types::chrono::NaiveDate::parse_from_str(&observation.as_of, "%Y-%m-%d")
        .map_err(|_| unavailable())?;
    let day = as_of
        .and_hms_opt(0, 0, 0)
        .ok_or_else(unavailable)?
        .and_utc()
        .timestamp_millis();
    let now = (state.wall)();
    if day > now.0 {
        return Err(unavailable());
    }
    let quote = SellerRuleRepo::new(state.pool.clone())
        .record_reference(
            context.org,
            &NewReferenceQuote {
                source: query.source,
                target: query.target,
                rate_micros: observation.rate_micros,
                as_of,
                provider: "ECB".to_owned(),
                source_url: ECB_DATA_URL.to_owned(),
                fetched_at: now,
            },
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(ReferenceView {
        id: Uuid(*quote.id.as_bytes()),
        source: quote.source,
        target: quote.target,
        rate: rate_string(quote.rate_micros),
        rate_micros: quote.rate_micros,
        as_of: quote.as_of.format("%Y-%m-%d").to_string(),
        provider: quote.provider,
        source_url: quote.source_url,
        fetched_at: quote.fetched_at,
        notice: ECB_NOTICE,
    }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ResourceSelection {
    All { all: bool },
    Products { products: Vec<ProductId> },
}

impl ResourceSelection {
    fn validate(&self) -> Result<(), APIError> {
        match self {
            Self::All { all: false } => Err(validation(
                "Select all resources or supply an explicit resource selection.",
            )),
            Self::Products { products } if products.is_empty() => {
                Err(validation("Select at least one resource."))
            }
            Self::All { all: true } | Self::Products { .. } => Ok(()),
        }
    }

    fn storage(&self) -> Selection {
        match self {
            Self::All { .. } => Selection::All,
            Self::Products { products } => Selection::Products(products.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    pub source: InventoryId,
    pub target: InventoryId,
    pub selection: ResourceSelection,
    pub rule_ids: Option<Vec<Uuid>>,
    pub draft: Option<SellerRuleDefinition>,
    #[serde(default)]
    pub overrides: RuleOverrides,
}

#[derive(Debug, Serialize)]
pub struct PreviewRow {
    pub product: ProductId,
    pub title: String,
    pub source_price: PriceIntent,
    pub before: TargetFields,
    pub proposed: TargetFields,
    pub matches: Vec<RuleMatch>,
    pub blockers: Vec<String>,
    pub status: &'static str,
    pub decision: &'static str,
}

#[derive(Debug, Default, Serialize)]
pub struct PreviewCounts {
    pub proposed: usize,
    pub unchanged: usize,
    pub blocked: usize,
}

impl PreviewCounts {
    fn include(&mut self, status: PreviewStatus) -> &'static str {
        match status {
            PreviewStatus::Proposed => {
                self.proposed += 1;
                "proposed"
            }
            PreviewStatus::Unchanged => {
                self.unchanged += 1;
                "unchanged"
            }
            PreviewStatus::Blocked => {
                self.blocked += 1;
                "blocked"
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RulePreview {
    pub id: Uuid,
    pub source: InventoryId,
    pub target: InventoryId,
    pub rows: Vec<PreviewRow>,
    pub counts: PreviewCounts,
}

async fn selected_rules(
    state: &AppState,
    context: &OrgContext,
    body: &PreviewRequest,
) -> Result<Vec<SellerRuleRecord>, APIError> {
    let repo = SellerRuleRepo::new(state.pool.clone());
    if let Some(ids) = &body.rule_ids {
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            if records
                .iter()
                .any(|record: &SellerRuleRecord| record.id == *id)
            {
                continue;
            }
            let rule = repo
                .get(context.org, uuid::Uuid::from_bytes(id.0))
                .await
                .map_err(|error| state.internal(&error.to_string()))?
                .ok_or_else(missing)?;
            if rule.definition.source != body.source || rule.definition.target != body.target {
                return Err(validation(
                    "Every selected rule must use the preview's source and target.",
                ));
            }
            records.push(rule);
        }
        return Ok(records);
    }
    let mut records = Vec::new();
    let mut page = 1;
    loop {
        let batch = repo
            .list(
                context.org,
                &RuleQuery {
                    kind: None,
                    state: RuleState::Enabled,
                    source: Some(body.source),
                    target: Some(body.target),
                    search: None,
                    page,
                    page_size: 100,
                },
            )
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
        records.extend(batch.rows);
        if !batch.has_next {
            break;
        }
        page = page
            .checked_add(1)
            .ok_or_else(|| validation("The rule selection is too large."))?;
    }
    Ok(records)
}

fn merge_fields(before: &TargetFields, patch: TargetFields) -> TargetFields {
    TargetFields {
        price: patch.price.or(before.price),
        licence: patch.licence.or_else(|| before.licence.clone()),
        resource_type: patch.resource_type.or_else(|| before.resource_type.clone()),
    }
}

pub(crate) async fn preview(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<PreviewRequest>,
) -> Result<Json<RulePreview>, APIError> {
    body.selection.validate()?;
    if body.source == body.target {
        return Err(validation(
            "Choose different source and target marketplaces.",
        ));
    }
    if let Some(draft) = &body.draft {
        if draft.source != body.source || draft.target != body.target {
            return Err(validation(
                "The draft rule must use the preview's source and target.",
            ));
        }
        validate_rule(&state, &context, draft).await?;
    }
    if let Some(rate) = &body.overrides.rate {
        rules::parse_rate(rate).map_err(|error| validation(&error.to_string()))?;
    }
    let captured = selected_rules(&state, &context, &body).await?;
    let mut evaluated = captured.clone();
    if body.rule_ids.is_some() {
        for rule in &mut evaluated {
            rule.definition.enabled = true;
        }
    }
    let now = (state.wall)();
    if let Some(draft) = &body.draft {
        let mut definition = draft.clone();
        definition.enabled = true;
        evaluated.push(SellerRuleRecord {
            id: fresh_id(),
            revision: 0,
            definition,
            author: context.user,
            created_at: now,
            updated_at: now,
        });
    }
    let products = ProductRepo::new(state.pool.clone());
    let mut selected = match &body.selection {
        ResourceSelection::All { .. } => products
            .list(context.org)
            .await
            .map_err(|error| state.internal(&error.to_string()))?
            .into_iter()
            .map(|row| row.id)
            .collect(),
        ResourceSelection::Products { products } => products.clone(),
    };
    selected.sort_by_key(|id| id.0 .0);
    selected.dedup();
    let mappings = tam_storage::MappingRepo::new(state.pool.clone());
    let source_members: std::collections::HashSet<_> = mappings
        .heads_for_products(context.org, body.source, &selected)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .into_iter()
        .filter(|head| head.remote.is_some())
        .map(|head| head.product)
        .collect();
    if matches!(body.selection, ResourceSelection::All { .. }) {
        selected.retain(|product| source_members.contains(product));
    }
    let repo = SellerRuleRepo::new(state.pool.clone());
    let mut stored_rows = Vec::with_capacity(selected.len());
    let mut missing_products = Vec::new();
    for product_id in selected {
        let Some(record) = products
            .get(context.org, product_id)
            .await
            .map_err(|error| state.internal(&error.to_string()))?
        else {
            missing_products.push(product_id);
            continue;
        };
        let previous = repo
            .latest_accepted_fields(context.org, product_id, body.target)
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
        let before = previous
            .as_ref()
            .map_or_else(TargetFields::default, AcceptedChoice::fields);
        let mut evaluation = rules::evaluate(
            &record.product,
            (body.source, body.target),
            &evaluated,
            &body.overrides,
            &before,
        );
        if !source_members.contains(&product_id) {
            let declared_source = matches!(
                record.product.grades.source,
                tam_domain::DeclarationSource::Imported { .. }
            );
            let listed_elsewhere = mappings
                .list_for_product(context.org, product_id)
                .await
                .map_err(|error| state.internal(&error.to_string()))?
                .into_iter()
                .any(|held| matches!(held.mapping.binding, tam_domain::Binding::Bound { .. }));
            if declared_source || listed_elsewhere {
                evaluation.blockers.push(format!(
                    "This resource is not listed in the selected {:?} source catalogue.",
                    body.source,
                ));
            }
        }
        let patch = evaluation.fields;
        let proposed = merge_fields(&before, patch.clone());
        let status = if !evaluation.blockers.is_empty() {
            PreviewStatus::Blocked
        } else if proposed == before {
            PreviewStatus::Unchanged
        } else {
            PreviewStatus::Proposed
        };
        stored_rows.push(NewPreviewRow {
            product: product_id,
            fingerprint: tam_storage::seller_rules::fingerprint(&record.product),
            previous_choice: previous.map(|choice| choice.id),
            title: record.product.title.0,
            source_price: record.product.price,
            before,
            proposed,
            patch,
            matches: evaluation.matches,
            blockers: evaluation.blockers,
            status,
        });
    }
    if !missing_products.is_empty() {
        return Err(validation(&format!(
            "These selected resources are unavailable; refresh the selection: {}",
            missing_products
                .iter()
                .map(|id| id.0.to_hyphenated())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let id = fresh_id();
    let request =
        serde_json::to_value(&body).map_err(|error| state.internal(&error.to_string()))?;
    repo.save_preview(
        context.org,
        NewPreview {
            id: uuid::Uuid::from_bytes(id.0),
            source: body.source,
            target: body.target,
            request: &request,
            scope: if body.rule_ids.is_some() {
                RuleScope::Explicit
            } else {
                RuleScope::Implicit
            },
            captured_rules: &captured,
            actor: context.user,
            at: now,
            rows: &stored_rows,
        },
    )
    .await
    .map_err(|error| state.internal(&error.to_string()))?;
    let mut counts = PreviewCounts::default();
    let rows = stored_rows
        .into_iter()
        .map(|row| {
            let status = counts.include(row.status);
            PreviewRow {
                product: row.product,
                title: row.title,
                source_price: row.source_price,
                before: row.before,
                proposed: row.proposed,
                matches: row.matches,
                blockers: row.blockers,
                status,
                decision: "pending",
            }
        })
        .collect();
    Ok(Json(RulePreview {
        id,
        source: body.source,
        target: body.target,
        rows,
        counts,
    }))
}

pub(crate) async fn read_preview(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, raw)): Path<(String, String)>,
) -> Result<Json<RulePreview>, APIError> {
    let held = SellerRuleRepo::new(state.pool.clone())
        .preview(context.org, parse_id(&raw)?)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .ok_or_else(missing)?;
    let mut counts = PreviewCounts::default();
    let rows = held
        .rows
        .into_iter()
        .map(|row| PreviewRow {
            product: row.product,
            title: row.title,
            source_price: row.source_price,
            before: row.before,
            proposed: row.proposed,
            matches: row.matches,
            blockers: row.blockers,
            status: counts.include(row.status),
            decision: match row.decision {
                PreviewDecision::Pending => "pending",
                PreviewDecision::Accepted => "accepted",
                PreviewDecision::Rejected => "rejected",
            },
        })
        .collect();
    Ok(Json(RulePreview {
        id: Uuid(*held.id.as_bytes()),
        source: held.source,
        target: held.target,
        rows,
        counts,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Accept,
    Reject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRequest {
    pub decision: Decision,
    pub selection: ResourceSelection,
}

#[derive(Debug, Serialize)]
pub struct DecisionView {
    pub accepted: usize,
    pub rejected: usize,
    pub remaining: usize,
    pub blocked: usize,
}

pub(crate) async fn decide(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, raw)): Path<(String, String)>,
    Json(body): Json<DecisionRequest>,
) -> Result<Json<DecisionView>, APIError> {
    body.selection.validate()?;
    let kind = match body.decision {
        Decision::Accept => DecisionKind::Accept,
        Decision::Reject => DecisionKind::Reject,
    };
    let selection = body.selection.storage();
    match SellerRuleRepo::new(state.pool.clone()).decide(context.org, &RuleDecision {
        preview: parse_id(&raw)?, actor: context.user, kind, selection: &selection, at: (state.wall)(),
    })
        .await.map_err(|error| state.internal(&error.to_string()))? {
        DecisionOutcome::Applied(report) => Ok(Json(DecisionView { accepted: report.accepted, rejected: report.rejected, remaining: report.remaining, blocked: report.blocked })),
        DecisionOutcome::UnknownPreview => Err(missing()),
        DecisionOutcome::Conflict(_) => Err(conflict("Some selected proposals already have the opposite decision. Create a new preview to change an accepted choice.")),
        DecisionOutcome::Stale(rows) => {
            let details = rows.into_iter().map(|row| {
                let reason = match row.reason {
                    StaleReason::SourceChanged => "source changed",
                    StaleReason::RuleRevised { .. } => "rule changed",
                    StaleReason::ChoiceChanged => "target choice changed",
                    StaleReason::Blocked => "proposal is blocked",
                    StaleReason::Missing => "resource is unavailable",
                };
                match row.product {
                    Some(product) => format!("{}: {reason}", product.0.to_hyphenated()),
                    None => reason.to_owned(),
                }
            }).collect::<Vec<_>>().join("; ");
            Err(conflict(&format!("Nothing was applied. Create a fresh preview: {details}")))
        }
    }
}
