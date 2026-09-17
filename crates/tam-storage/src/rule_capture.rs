//! Freeze exact target prices and native terms in the transaction that queues
//! each non-remove item. Claims read that output, never current policy.
//!
//! The job's origin determines its policy scope:
//! - Confirmed requests freeze outputs for known products and policies for
//!   source resources not yet imported.
//! - Jobs without a request use current accepted choices and rules explicitly
//!   opted into cross-listing. Manual choices win field by field.
//! - A request without a snapshot predates this feature. It retains canonical
//!   pricing without inventing a seller approval or applying new rules.
//!
//! Standing policy reads share the tenant policy lock with edits and approvals;
//! confirmed requests read immutable snapshots.

use sqlx::{PgPool, Postgres, Transaction};
use tam_domain::seller_rules::{
    evaluate, FrozenRuleOutput, RuleAction, RuleMatch, RuleOverrides, RuleUse, SellerRuleRecord,
    SellerTermChoice, TargetFields,
};
use tam_domain::{CanonicalProduct, ItemOperation, JobItemId};
use tam_marketplace::{idempotency::derive_idempotency_key, IdempotencyKey};
use tam_types::{InventoryId, MappingId, OrgId, PriceIntent, ProductId, Uuid};

use crate::codec::{inventory_from_db, inventory_to_db, uuid_from_db, uuid_to_db};
use crate::seller_rules::{self, AcceptedChoice, RequestChoice, RequestRuleSnapshot};
use crate::{pin_org, StorageError};

/// Domain separator for the frozen-output generation of an item key, beside
/// `NON_CREATE_INTENT_DOMAIN` and `SEVERED_CREATE_DOMAIN` in
/// [`crate::job_reads`] and for the same reason: no encoding in one generation
/// can be read as bytes from another.
const FROZEN_OUTPUT_DOMAIN: &[u8] = b"tam.item.intent.frozen.v1\x00";

/// The intent version every item carrying a frozen output derives under.
///
/// A new version rather than a re-hash under version one, because the two
/// answer different questions: version one asks "are these the same files",
/// and this asks "are these the same files *for the same approved money and
/// terms*". Existing rows keep the keys they were written with and are never
/// re-derived — nothing reads a key to decide anything except the uniqueness
/// index, and re-deriving would either refuse an in-flight item or admit a
/// second copy of it.
///
/// This is what lets a corrected create be queued at all. A seller whose
/// £2.99 create was rejected corrects it to £3.49; the files are unchanged, so
/// the payload digest and therefore the base key are identical, and the old
/// key would be refused forever by `job_item_idempotent` with a message about
/// unchanged content that is false. Mixing the approved output separates them,
/// and mixing *only* the approved output keeps the no-op property for an
/// ordinary re-sync whose price and terms did not move.
pub const FROZEN_INTENT_VERSION: u32 = 2;

/// One item's frozen output and the key that output produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frozen {
    pub(crate) key: IdempotencyKey,
    pub(crate) output: FrozenRuleOutput,
}

/// The fields a capture resolved, before they become a [`FrozenRuleOutput`].
///
/// Separate from `TargetFields` because a frozen term carries the person who
/// chose it: a licence reaching a marketplace is a grant, and a grant with no
/// author is one nobody made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Resolved {
    price: Option<PriceIntent>,
    licence: Option<SellerTermChoice>,
    resource_type: Option<SellerTermChoice>,
    /// Only the rules that actually supplied a field above. A rule that
    /// matched and lost every field to a manual choice is not provenance for
    /// anything, and recording it would show the seller a rule deciding money
    /// their own accepted choice decided.
    matches: Vec<RuleMatch>,
}

impl Resolved {
    fn from_choice(choice: Option<&AcceptedChoice>) -> Self {
        let Some(choice) = choice else {
            return Self::default();
        };
        Self {
            price: choice.price,
            licence: choice.licence.clone(),
            resource_type: choice.resource_type.clone(),
            matches: choice.matches.clone(),
        }
    }

    fn fields(&self) -> TargetFields {
        TargetFields {
            price: self.price,
            licence: self.licence.as_ref().map(|choice| choice.native_id.clone()),
            resource_type: self
                .resource_type
                .as_ref()
                .map(|choice| choice.native_id.clone()),
        }
    }

    /// Every field a human or a rule could supply is supplied, so no further
    /// rule needs consulting.
    fn complete(&self) -> bool {
        self.price.is_some() && self.licence.is_some() && self.resource_type.is_some()
    }

    fn into_output(self, canonical: PriceIntent) -> FrozenRuleOutput {
        FrozenRuleOutput {
            // Every non-remove item gets an exact price even where no rule
            // and no choice named one: the canonical price at the instant the
            // seller asked, rather than whatever the product says when a
            // device finally claims the item.
            price: self.price.unwrap_or(canonical),
            licence: self.licence,
            resource_type: self.resource_type,
            matches: self.matches,
        }
    }
}

enum SnapshotCache {
    Unread,
    Legacy,
    Confirmed(RequestRuleSnapshot),
}

/// The per-job freeze, holding what is a job-level fact so a five-hundred-item
/// publish does not ask the same question five hundred times.
///
/// The rule lists and the request snapshot are per job; the product and its
/// accepted choice are per item and are read per item.
pub(crate) struct RuleCapture {
    org: OrgId,
    target: InventoryId,
    request: Option<Uuid>,
    snapshot: SnapshotCache,
    standing: Vec<(InventoryId, Vec<SellerRuleRecord>)>,
    direction: Option<InventoryId>,
    use_: RuleUse,
    policy_locked: bool,
}

impl RuleCapture {
    /// The scope, from the job's origin. `request` is the `sync_request` the
    /// job belongs to, which both mint paths already name in the same
    /// statement that inserts the job row.
    pub(crate) const fn for_job(org: OrgId, target: InventoryId, request: Option<Uuid>) -> Self {
        Self {
            org,
            target,
            request,
            snapshot: SnapshotCache::Unread,
            standing: Vec::new(),
            direction: None,
            use_: RuleUse::CrossList,
            policy_locked: false,
        }
    }

    fn for_scope(org: OrgId, scope: PricingScope) -> Self {
        let (source, target, use_) = match scope {
            PricingScope::CrossList(target) => (None, target, RuleUse::CrossList),
            PricingScope::Transfer {
                source,
                target,
                use_,
            } => (Some(source), target, use_),
        };
        Self {
            direction: source,
            use_,
            ..Self::for_job(org, target, None)
        }
    }

    /// The output this item will post, and the key it derives.
    ///
    /// `None` for a removal: a removal carries no price and no terms, it needs
    /// no freeze, and keying it on one would re-key every removal in flight
    /// for no gain.
    pub(crate) async fn freeze(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        mapping: MappingId,
        operation: &ItemOperation,
        base: IdempotencyKey,
    ) -> Result<Option<Frozen>, StorageError> {
        if matches!(operation, ItemOperation::Remove { .. }) {
            return Ok(None);
        }
        // A mapping that does not exist freezes nothing, because the insert
        // this keys is an `INSERT ... SELECT FROM mapping` and stores no row
        // either: the two agree on doing nothing rather than this one raising
        // a fault over a row the statement itself treats as absent.
        let Some(product) = product_of_mapping(tx, self.org, mapping).await? else {
            return Ok(None);
        };
        // In this transaction rather than through a pooled read, because the
        // canonicalisation that produced this product may be uncommitted: a
        // device's completing page canonicalises and mints in one
        // transaction, and a second connection would evaluate the rules
        // against a product that does not exist yet.
        let record = crate::product::get_product_in_tx(tx, self.org, product)
            .await?
            .ok_or_else(|| StorageError::Inconsistent {
                reason: "the product a queued item maps to must exist".to_owned(),
            })?;
        let canonical = record.product.price;
        let resolved = match self.request {
            Some(request) => {
                self.confirmed(tx, request, product, &record.product)
                    .await?
            }
            None => self.standing(tx, product, &record.product).await?,
        };
        let output = resolved.into_output(canonical);
        let key = keyed(self.org, self.target, product, base, &output);
        Ok(Some(Frozen { key, output }))
    }

    /// The confirmed scope: the definitions and per-resource choices captured
    /// when the seller confirmed this request, evaluated now that the source
    /// read has canonicalised the product.
    async fn confirmed(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        request: Uuid,
        product: ProductId,
        canonical: &CanonicalProduct,
    ) -> Result<Resolved, StorageError> {
        if matches!(self.snapshot, SnapshotCache::Unread) {
            self.snapshot =
                match seller_rules::request_snapshot_in_tx(tx, self.org, uuid_to_db(request))
                    .await?
                {
                    Some(snapshot) => SnapshotCache::Confirmed(snapshot),
                    None => SnapshotCache::Legacy,
                };
        }
        let snapshot = match &self.snapshot {
            SnapshotCache::Confirmed(snapshot) => snapshot,
            SnapshotCache::Legacy => return Ok(Resolved::default()),
            SnapshotCache::Unread => {
                return Err(StorageError::Inconsistent {
                    reason: "a confirmed request snapshot has not been loaded".to_owned(),
                });
            }
        };
        if snapshot.target != self.target {
            return Err(StorageError::Inconsistent {
                reason: "a request's captured target must match its queued job".to_owned(),
            });
        }
        let choice = snapshot
            .choices
            .iter()
            .find(|(subject, _)| *subject == product)
            .map(|(_, choice)| choice);
        match choice {
            Some(RequestChoice::Output(output)) => Ok(Resolved {
                price: Some(output.price),
                licence: output.licence.clone(),
                resource_type: output.resource_type.clone(),
                matches: output.matches.clone(),
            }),
            Some(RequestChoice::Accepted(accepted)) => proposals(
                &Resolved::from_choice(Some(accepted)),
                canonical,
                snapshot.source,
                self.target,
                &snapshot.rules,
            ),
            None => {
                if snapshot
                    .choices
                    .iter()
                    .any(|(_, choice)| matches!(choice, RequestChoice::Output(_)))
                {
                    return Err(StorageError::Inconsistent {
                        reason: "the resource is outside the confirmed catalogue selection"
                            .to_owned(),
                    });
                }
                proposals(
                    &Resolved::default(),
                    canonical,
                    snapshot.source,
                    self.target,
                    &snapshot.rules,
                )
            }
        }
    }

    /// The standing scope: the seller's latest accepted choice for this
    /// target, then the definitions they opted into automatic cross-listing.
    ///
    /// Only recorded source directions are eligible. Proposals from different
    /// eligible sources must agree; inventory ordering never chooses a grant.
    async fn standing(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        product: ProductId,
        canonical: &CanonicalProduct,
    ) -> Result<Resolved, StorageError> {
        if !self.policy_locked {
            seller_rules::lock_policy(tx, self.org).await?;
            self.policy_locked = true;
        }
        let target = self.target;
        let accepted =
            seller_rules::latest_accepted_fields_in_tx(tx, self.org, product, target).await?;
        let base = Resolved::from_choice(accepted.as_ref());
        let mut resolved = proposals(&base, canonical, target, target, &[])?;
        if base.complete() {
            return Ok(resolved);
        }
        let sources = match self.direction {
            Some(source) => vec![source],
            None => source_inventories(tx, self.org, canonical, target).await?,
        };
        for source in sources {
            let rules = self.standing_rules(tx, source).await?;
            let proposed = proposals(&base, canonical, source, target, rules)?;
            merge_proposals(&mut resolved, proposed, product)?;
        }
        Ok(resolved)
    }

    /// The enabled definitions opted into automatic cross-listing for one
    /// direction, read once per job and held for every later item of it.
    async fn standing_rules(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        source: InventoryId,
    ) -> Result<&[SellerRuleRecord], StorageError> {
        if !self.standing.iter().any(|(cached, _)| *cached == source) {
            let rules =
                seller_rules::enabled_rules_in_tx(tx, self.org, source, self.target, self.use_)
                    .await?;
            self.standing.push((source, rules));
        }
        Ok(self
            .standing
            .iter()
            .find(|(cached, _)| *cached == source)
            .map_or(&[][..], |(_, rules)| rules.as_slice()))
    }
}

/// Evaluate automatic fields without letting them overrule explicit choices.
fn proposals(
    accepted: &Resolved,
    product: &CanonicalProduct,
    source: InventoryId,
    target: InventoryId,
    rules: &[SellerRuleRecord],
) -> Result<Resolved, StorageError> {
    let base = accepted.fields();
    let overrides = RuleOverrides {
        price: base.price,
        rate: None,
        licence: base.licence.clone(),
        resource_type: base.resource_type.clone(),
    };
    let evaluation = evaluate(product, (source, target), rules, &overrides, &base);
    if !evaluation.blockers.is_empty() {
        return Err(StorageError::SellerRuleBlocked {
            product: product.id,
            reasons: evaluation.blockers,
        });
    }
    let mut resolved = accepted.clone();
    if resolved.price.is_none() {
        resolved.price = evaluation.fields.price;
    }
    if resolved.licence.is_none() {
        resolved.licence = authored(
            rules,
            &evaluation.matches,
            Term::Licence,
            evaluation.fields.licence.as_deref(),
        );
        if evaluation.fields.licence.is_some() && resolved.licence.is_none() {
            return Err(StorageError::Inconsistent {
                reason: "an automatic licence must name its rule author".to_owned(),
            });
        }
    }
    if resolved.resource_type.is_none() {
        resolved.resource_type = authored(
            rules,
            &evaluation.matches,
            Term::ResourceType,
            evaluation.fields.resource_type.as_deref(),
        );
        if evaluation.fields.resource_type.is_some() && resolved.resource_type.is_none() {
            return Err(StorageError::Inconsistent {
                reason: "an automatic resource type must name its rule author".to_owned(),
            });
        }
    }
    for matched in evaluation.matches {
        let contributes = rules
            .iter()
            .find(|rule| rule.id == matched.id)
            .is_some_and(|rule| match &rule.definition.action {
                RuleAction::Pricing { .. } => accepted.price.is_none() && resolved.price.is_some(),
                RuleAction::Mapping {
                    licence,
                    resource_type,
                } => {
                    (accepted.licence.is_none()
                        && licence.as_ref().is_some_and(|native| {
                            resolved
                                .licence
                                .as_ref()
                                .is_some_and(|choice| choice.native_id == *native)
                        }))
                        || (accepted.resource_type.is_none()
                            && resource_type.as_ref().is_some_and(|native| {
                                resolved
                                    .resource_type
                                    .as_ref()
                                    .is_some_and(|choice| choice.native_id == *native)
                            }))
                }
            });
        if contributes && !resolved.matches.iter().any(|held| held.id == matched.id) {
            resolved.matches.push(matched);
        }
    }
    Ok(resolved)
}

fn merge_proposals(
    held: &mut Resolved,
    next: Resolved,
    product: ProductId,
) -> Result<(), StorageError> {
    let differs = match (held.price, next.price) {
        (Some(left), Some(right)) => left != right,
        _ => false,
    } || match (&held.licence, &next.licence) {
        (Some(left), Some(right)) => left.native_id != right.native_id,
        _ => false,
    } || match (&held.resource_type, &next.resource_type) {
        (Some(left), Some(right)) => left.native_id != right.native_id,
        _ => false,
    };
    if differs {
        return Err(StorageError::SellerRuleBlocked {
            product,
            reasons: vec![
                "Automatic rules from this resource's source marketplaces disagree. Preview and approve an explicit price or mapping.".to_owned(),
            ],
        });
    }
    held.price = held.price.or(next.price);
    held.licence = held.licence.take().or(next.licence);
    held.resource_type = held.resource_type.take().or(next.resource_type);
    for matched in next.matches {
        if !held.matches.iter().any(|current| current.id == matched.id) {
            held.matches.push(matched);
        }
    }
    Ok(())
}

async fn source_inventories(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    product: &CanonicalProduct,
    target: InventoryId,
) -> Result<Vec<InventoryId>, StorageError> {
    let rows = sqlx::query_scalar!(
        "SELECT DISTINCT inventory FROM mapping \
         WHERE org_id = $1 AND product_id = $2 AND binding_state = 'bound' AND inventory <> $3",
        uuid_to_db(org.0),
        uuid_to_db(product.id.0),
        inventory_to_db(target),
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut sources = rows
        .iter()
        .map(|raw| inventory_from_db(raw))
        .collect::<Result<Vec<_>, _>>()?;
    let mut retain = |source| {
        if source != target && !sources.contains(&source) {
            sources.push(source);
        }
    };
    if let tam_domain::DeclarationSource::Imported { vocabulary } = product.grades.source {
        retain(vocabulary.0);
    }
    if let tam_domain::RightsDeclaration::Declared { source } = &product.rights {
        retain(source.vocabulary.0);
    }
    for residue in &product.native_residue {
        retain(residue.inventory);
    }
    Ok(sources)
}

/// Which native field a mapping action supplies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Term {
    Licence,
    ResourceType,
}

/// The rule author behind an evaluated native value.
///
/// A licence or a resource type reaching a marketplace is a seller's own
/// grant, so the frozen output names the person who wrote the rule that
/// supplied it. Where no matched rule accounts for the value the field is left
/// unset rather than attributed to nobody: an unattributed grant is one the
/// projection would deliver on no authority at all, and the contract puts
/// human provenance on every accepted term.
fn authored(
    rules: &[SellerRuleRecord],
    matches: &[RuleMatch],
    term: Term,
    value: Option<&str>,
) -> Option<SellerTermChoice> {
    let native = value?;
    let author = matches.iter().rev().find_map(|matched| {
        let record = rules.iter().find(|rule| rule.id == matched.id)?;
        let RuleAction::Mapping {
            licence,
            resource_type,
        } = &record.definition.action
        else {
            return None;
        };
        let supplied = match term {
            Term::Licence => licence.as_deref(),
            Term::ResourceType => resource_type.as_deref(),
        };
        (supplied == Some(native)).then_some(record.author)
    })?;
    Some(SellerTermChoice {
        native_id: native.to_owned(),
        author,
    })
}

/// The key an item carrying a frozen output is stored under: the base key the
/// enqueuer derived, mixed with what the item will actually post.
///
/// The base key rather than the base key's inputs, because the enqueuer owns
/// that derivation — `intent_digest` decides what "the same write" means for
/// each operation — and a second assembly of those inputs here would be a
/// second answer to that question.
fn keyed(
    org: OrgId,
    inventory: InventoryId,
    product: ProductId,
    base: IdempotencyKey,
    output: &FrozenRuleOutput,
) -> IdempotencyKey {
    let material = output.digest_material();
    let mut encoded = Vec::with_capacity(FROZEN_OUTPUT_DOMAIN.len() + 16 + material.len());
    encoded.extend_from_slice(FROZEN_OUTPUT_DOMAIN);
    encoded.extend_from_slice(&base.0 .0);
    encoded.extend_from_slice(material.as_bytes());
    derive_idempotency_key(
        org,
        inventory,
        product,
        FROZEN_INTENT_VERSION,
        tam_pipeline::hash::content_hash(&encoded),
    )
}

/// The product one mapping names, in the caller's transaction, or `None`
/// where the mapping does not exist — which the insert this serves treats the
/// same way, by selecting no row to store.
async fn product_of_mapping(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    mapping: MappingId,
) -> Result<Option<ProductId>, StorageError> {
    let found = sqlx::query_scalar!(
        "SELECT product_id FROM mapping WHERE org_id = $1 AND id = $2",
        uuid_to_db(org.0),
        uuid_to_db(mapping.0),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(found.map(|id| ProductId(uuid_from_db(id))))
}

/// Capture after inserting the request, in that same transaction.
pub(crate) struct RequestCapture<'a> {
    pub request: Uuid,
    pub source: InventoryId,
    pub target: InventoryId,
    pub use_: RuleUse,
    /// A native source read has not discovered its product IDs yet.
    pub products: Option<&'a [ProductId]>,
    pub at: tam_types::Timestamp,
}

pub(crate) async fn confirm_request_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    capture: &RequestCapture<'_>,
) -> Result<(), StorageError> {
    seller_rules::lock_policy(tx, org).await?;
    let rules =
        seller_rules::enabled_rules_in_tx(tx, org, capture.source, capture.target, capture.use_)
            .await?;
    let choices =
        seller_rules::accepted_choices_in_tx(tx, org, capture.target, capture.products).await?;
    let choices = if let Some(products) = capture.products {
        let accepted: std::collections::HashMap<_, _> = choices.into_iter().collect();
        let mut products = products.to_vec();
        products.sort_by_key(|product| product.0 .0);
        products.dedup();
        let mut outputs = Vec::with_capacity(products.len());
        for product in products {
            crate::product::lock_product(tx, uuid_to_db(org.0), uuid_to_db(product.0)).await?;
            let record = crate::product::get_product_in_tx(tx, org, product)
                .await?
                .ok_or_else(|| StorageError::Inconsistent {
                    reason: "a confirmed catalogue resource must still exist".to_owned(),
                })?;
            let resolved = proposals(
                &Resolved::from_choice(accepted.get(&product)),
                &record.product,
                capture.source,
                capture.target,
                &rules,
            )?;
            outputs.push((
                product,
                RequestChoice::Output(resolved.into_output(record.product.price)),
            ));
        }
        outputs
    } else {
        choices
            .into_iter()
            .map(|(product, choice)| (product, RequestChoice::Accepted(choice)))
            .collect()
    };
    seller_rules::save_request_snapshot_in_tx(
        tx,
        org,
        uuid_to_db(capture.request),
        &seller_rules::NewRequestSnapshot {
            source: capture.source,
            target: capture.target,
            rules: &rules,
            choices: &choices,
            at: capture.at,
        },
    )
    .await
}

/// The scope a request's disposition puts it in.
///
/// A copy leaves the source listing standing and a move takes it down, and a
/// seller opts a rule into each separately: the price they want on a copy is
/// not necessarily the price they want on the listing that replaces their
/// only one.
#[must_use]
pub const fn scope_of(disposition: crate::sync_requests::Disposition) -> RuleUse {
    match disposition {
        crate::sync_requests::Disposition::Sync => RuleUse::Copy,
        crate::sync_requests::Disposition::Migrate => RuleUse::Move,
    }
}

/// The operation whose opted-in rules an admission or export will evaluate.
#[derive(Debug, Clone, Copy)]
pub enum PricingScope {
    CrossList(InventoryId),
    Transfer {
        source: InventoryId,
        target: InventoryId,
        use_: RuleUse,
    },
}

/// The same accepted and automatic fields that enqueue would capture now.
pub async fn approved_fields(
    pool: &PgPool,
    org: OrgId,
    product: ProductId,
    scope: PricingScope,
) -> Result<TargetFields, StorageError> {
    let mut tx = pool.begin().await?;
    pin_org(&mut tx, org).await?;
    let mut capture = RuleCapture::for_scope(org, scope);
    let Some(record) = crate::product::get_product_in_tx(&mut tx, org, product).await? else {
        return Ok(TargetFields::default());
    };
    let resolved = capture.standing(&mut tx, product, &record.product).await?;
    tx.commit().await?;
    Ok(resolved.fields())
}

/// Preflight a native read under the policy captured before the read began.
pub async fn confirmed_output(
    pool: &PgPool,
    org: OrgId,
    request: Uuid,
    product: &CanonicalProduct,
) -> Result<Option<FrozenRuleOutput>, StorageError> {
    let mut tx = pool.begin().await?;
    pin_org(&mut tx, org).await?;
    let Some(snapshot) =
        seller_rules::request_snapshot_in_tx(&mut tx, org, uuid_to_db(request)).await?
    else {
        tx.commit().await?;
        return Ok(None);
    };
    let target = snapshot.target;
    let mut capture = RuleCapture {
        snapshot: SnapshotCache::Confirmed(snapshot),
        ..RuleCapture::for_job(org, target, Some(request))
    };
    let resolved = capture
        .confirmed(&mut tx, request, product.id, product)
        .await?;
    tx.commit().await?;
    Ok(Some(resolved.into_output(product.price)))
}

/// Preflight a new canonical product before its import transaction persists it.
pub async fn prospective_output(
    pool: &PgPool,
    org: OrgId,
    product: &CanonicalProduct,
    scope: PricingScope,
) -> Result<FrozenRuleOutput, StorageError> {
    let mut tx = pool.begin().await?;
    pin_org(&mut tx, org).await?;
    let mut capture = RuleCapture::for_scope(org, scope);
    let resolved = capture.standing(&mut tx, product.id, product).await?;
    tx.commit().await?;
    Ok(resolved.into_output(product.price))
}

/// The output one item was enqueued to post, read by the engine at claim
/// time.
///
/// Typed on [`JobItemId`] rather than on a bare database uuid, because the
/// engine holds the former and the conversion belongs on this side of the
/// boundary with the rest of the codec.
pub async fn frozen_output(
    pool: &PgPool,
    org: OrgId,
    item: JobItemId,
) -> Result<Option<FrozenRuleOutput>, StorageError> {
    seller_rules::item_output(pool, org, uuid_to_db(item.0)).await
}

/// The approved price for one resource and target where the seller has
/// approved one, for the gates that must admit a conversion they authorised.
pub async fn approved_price(
    pool: &PgPool,
    org: OrgId,
    product: ProductId,
    scope: PricingScope,
) -> Result<Option<PriceIntent>, StorageError> {
    Ok(approved_fields(pool, org, product, scope).await?.price)
}

/// The same question for a page of resources, in one transaction, so an
/// admission or an export over five hundred rows does not open five hundred.
pub async fn approved_prices(
    pool: &PgPool,
    org: OrgId,
    products: &[ProductId],
    scope: PricingScope,
) -> Result<Vec<(ProductId, PriceIntent)>, StorageError> {
    let mut tx = pool.begin().await?;
    pin_org(&mut tx, org).await?;
    let mut capture = RuleCapture::for_scope(org, scope);
    let mut found = Vec::new();
    for product in products {
        let Some(record) = crate::product::get_product_in_tx(&mut tx, org, *product).await? else {
            continue;
        };
        let resolved = capture.standing(&mut tx, *product, &record.product).await?;
        if let Some(price) = resolved.price {
            found.push((*product, price));
        }
    }
    tx.commit().await?;
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::{keyed, FROZEN_INTENT_VERSION};
    use tam_domain::seller_rules::FrozenRuleOutput;
    use tam_marketplace::IdempotencyKey;
    use tam_types::{Currency, InventoryId, Money, OrgId, PriceIntent, ProductId, Uuid};

    const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
    const PRODUCT: ProductId = ProductId(Uuid([0x01; 16]));
    const BASE: IdempotencyKey = IdempotencyKey(Uuid([0x31; 16]));

    fn output(minor_units: i64) -> FrozenRuleOutput {
        FrozenRuleOutput {
            price: PriceIntent::Paid(
                Money::new(minor_units, Currency::Gbp).expect("a positive price"),
            ),
            licence: None,
            resource_type: None,
            matches: vec![],
        }
    }

    /// A re-sync of unchanged content under an unchanged approval is still
    /// the no-op the content-addressed key was built to be.
    #[test]
    fn the_same_approved_output_reproduces_the_same_key() {
        assert_eq!(
            keyed(ORG, InventoryId::Tes, PRODUCT, BASE, &output(22_500)),
            keyed(ORG, InventoryId::Tes, PRODUCT, BASE, &output(22_500)),
            "a requeued item must recompute the same key or lose its identity"
        );
    }

    /// The refusal this mixing exists to remove: a create rejected at £2.99
    /// and corrected to £3.49 has the same files, so the same base key, and
    /// under that key alone the correction is refused forever as unchanged
    /// content.
    #[test]
    fn a_corrected_price_is_a_different_write() {
        assert_ne!(
            keyed(ORG, InventoryId::Tes, PRODUCT, BASE, &output(299)),
            keyed(ORG, InventoryId::Tes, PRODUCT, BASE, &output(349)),
            "two approved prices for one unchanged payload are two writes"
        );
    }

    /// The base key still decides what the operation is. Two operations that
    /// agree on the money are still two items.
    #[test]
    fn the_base_key_still_separates_two_intents() {
        let other = IdempotencyKey(Uuid([0x32; 16]));
        assert_ne!(
            keyed(ORG, InventoryId::Tes, PRODUCT, BASE, &output(22_500)),
            keyed(ORG, InventoryId::Tes, PRODUCT, other, &output(22_500)),
            "the enqueuer's own derivation must still reach the stored key"
        );
    }

    /// No item already in the ledger is re-keyed onto a frozen key, and no
    /// frozen item lands on a historical one.
    #[test]
    fn a_frozen_key_is_never_a_key_the_ledger_already_holds() {
        assert_ne!(
            keyed(ORG, InventoryId::Tes, PRODUCT, BASE, &output(22_500)),
            BASE,
            "mixing must move the key off the generation existing rows were written under"
        );
        assert_eq!(
            FROZEN_INTENT_VERSION, 2,
            "version one is the generation of every row already in the ledger"
        );
    }
}
