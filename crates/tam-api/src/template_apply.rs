//! Applying a saved template to resources the catalogue already holds.
//!
//! A template prefills a blank form, and that is what it was for. This is the
//! other half a seller asks for the moment they have written one: forty
//! resources imported from a shop that carried a title, a price and nothing
//! else, and one template that says what the rest of the form should say.
//!
//! Two routes, and the first writes nothing, for `migrations.rs`'s reason: the
//! seller is told per resource whether anything will change, which fields, and
//! where it will not, before they spend a single write. The submit re-runs the
//! same plan rather than trusting the client's copy of it, so the two cannot
//! disagree about what was admitted.
//!
//! The merge is fill-empty. A template fills a field the resource has not
//! answered and leaves one it has, because a seller applying a template to
//! their catalogue is finishing the forms rather than rewriting them — and the
//! one field it never fills is the title, which is the resource's identity and
//! is not the sort of thing a template knows. `overwrite` widens "empty" to
//! "any", and is the seller's own tick.
//!
//! Idempotent by construction rather than by a key: a field is listed, and
//! written, only where the value the merge arrives at differs from the value
//! stored. So a replayed apply changes nothing and says so, `overwrite` or
//! not, and the `Idempotency-Key` the submit requires is the console's handle
//! on a retry rather than a fence this route needs.

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_authoring::{RefusalView, StandardInput};
use tam_domain::CanonicalProduct;
use tam_storage::{
    MappingRecord, MappingRepo, ProductRepo, ResourceCollectionRepo, ResourceTemplateRepo,
    TptBaseRepo,
};
use tam_types::{Currency, InventoryId, Money, OrgId, PriceIntent, ProductId, TermKind, Timestamp};

use crate::catalogue::{
    commit_edit, prepare_edit, stored_grade_slug, uncaptured_edits, EditBar, PatchProductBody,
    PathInput, PreparedEdit, UNCAPTURED_EDIT,
};
use crate::error::APIError;
use crate::jobs::{storage_fault, validation, RequestKey};
use crate::product::{DraftInput, TptBaseInput};
use crate::resources::tpt_base_input;
use crate::{AppState, OrgContext};

// -------------------------------------------------------------- vocabulary

/// What the preview says about one resource.
///
/// Three answers, and the seller's decision differs for each: a blocked row is
/// theirs to fix, an unchanged row is nothing to do, and only a will-change row
/// writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateApplyVerdict {
    WillChange,
    Unchanged,
    Blocked,
}

impl TemplateApplyVerdict {
    /// The closed set, in a stable order, for the vocabulary generator.
    pub const ALL: [Self; 3] = [Self::WillChange, Self::Unchanged, Self::Blocked];
}

// ------------------------------------------------------------------- wire

/// What the seller ticked, or the collection they were looking at.
///
/// Two spellings rather than one list, because a collection is a name the
/// seller maintains and expanding it in the browser would apply the template
/// to whatever the tab held rather than to what the collection holds now.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ApplySelection {
    Collection { collection: tam_types::Uuid },
    Products { products: Vec<ProductId> },
}

/// The one body both routes take, so the preview and the confirm cannot
/// describe two different applications.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApplyBody {
    pub selection: ApplySelection,
    /// Whether a field the resource has already answered is replaced.
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateApplyRow {
    pub product: ProductId,
    pub title: String,
    pub verdict: TemplateApplyVerdict,
    /// The fields this apply would write, named as the draft names them.
    /// Empty for an unchanged row and for a blocked one.
    pub fields: Vec<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TemplateApplyCounts {
    pub will_change: u32,
    pub unchanged: u32,
    pub blocked: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateApplyPlanView {
    pub rows: Vec<TemplateApplyRow>,
    pub counts: TemplateApplyCounts,
}

/// What the submit did, counted the way the plan counted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateApplyAck {
    pub changed: u32,
    pub unchanged: u32,
    pub blocked: u32,
}

// ------------------------------------------------------------------ handlers

/// The preview. Reads the template, the selection, each resource and its
/// sidecar, and writes nothing.
pub(crate) async fn plan_apply(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, template)): Path<(String, String)>,
    Json(body): Json<ApplyBody>,
) -> Result<Json<TemplateApplyPlanView>, APIError> {
    let plan = plan(&state, &context, &template, &body).await?;
    Ok(Json(plan.view))
}

/// The confirm. Re-plans and writes exactly the rows the plan admitted, one
/// resource at a time through the edit route's own write path.
///
/// One at a time rather than one transaction, deliberately: a resource whose
/// sidecar the merge makes unsubmittable is blocked by the plan, so the rows
/// that reach here are ones the create form's rules already accept, and a
/// single fault part way through leaves the resources before it finished
/// rather than rolling back work the seller can see was done. The counts say
/// how many.
pub(crate) async fn apply(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, template)): Path<(String, String)>,
    _key: RequestKey,
    Json(body): Json<ApplyBody>,
) -> Result<Json<TemplateApplyAck>, APIError> {
    let plan = plan(&state, &context, &template, &body).await?;
    let now = (state.wall)();
    let mut changed = 0u32;
    for admitted in &plan.admitted {
        if commit_edit(
            &state,
            context.org,
            admitted.product,
            &admitted.prepared,
            now,
        )
        .await?
        {
            changed = changed.saturating_add(1);
        }
    }
    Ok(Json(TemplateApplyAck {
        changed,
        unchanged: plan.view.counts.unchanged,
        // A resource deleted between the plan and the write is neither
        // changed nor blocked by anything the seller can act on, so it counts
        // where the plan put it and the count of writes is what moved.
        blocked: plan
            .view
            .counts
            .blocked
            .saturating_add(plan.view.counts.will_change.saturating_sub(changed)),
    }))
}

// --------------------------------------------------------------- the plan

struct Admitted {
    product: ProductId,
    prepared: PreparedEdit,
}

struct Plan {
    view: TemplateApplyPlanView,
    admitted: Vec<Admitted>,
}

async fn plan(
    state: &AppState,
    context: &OrgContext,
    template: &str,
    body: &ApplyBody,
) -> Result<Plan, APIError> {
    let id = uuid::Uuid::parse_str(template.trim())
        .map(|parsed| tam_types::Uuid(*parsed.as_bytes()))
        .map_err(|_unused| validation("We can't find that template."))?;
    let held = ResourceTemplateRepo::new(state.pool.clone())
        .get(context.org, id)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| validation("We can't find that template."))?;
    // The column holds what the template route validated as a draft, so a
    // document that will not read back as one is this tree's fault rather
    // than the caller's.
    let draft: DraftInput = serde_json::from_value(held.draft)
        .map_err(|error| state.internal(&format!("a stored template is not a draft: {error}")))?;

    let products = ProductRepo::new(state.pool.clone());
    let chosen = chosen_products(state, context.org, &body.selection).await?;
    let mappings = MappingRepo::new(state.pool.clone());
    let sidecars = TptBaseRepo::new(state.pool.clone());

    let mut rows = Vec::with_capacity(chosen.len());
    let mut admitted = Vec::new();
    let mut counts = TemplateApplyCounts::default();
    for product in chosen {
        // A named resource the catalogue no longer holds is dropped rather
        // than refused, which is `chosen_products`' rule for a stale tab.
        let Some(stored) = products
            .get(context.org, product)
            .await
            .map_err(|error| storage_fault(state, &error))?
        else {
            continue;
        };
        let stored = stored.product;
        let title = stored.title.0.clone();
        let bound = mappings
            .list_for_product(context.org, product)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        // The edit route's own refusal, reported as one row rather than as a
        // whole failed request: a seller applying a template to forty
        // resources has to see which of them their Tes listings block.
        if !uncaptured_edits(&bound).is_empty() {
            rows.push(blocked(product, title, UNCAPTURED_EDIT.to_owned()));
            counts.blocked = counts.blocked.saturating_add(1);
            continue;
        }
        let base = sidecars
            .get(context.org, product)
            .await
            .map_err(|error| storage_fault(state, &error))?
            .map(|record| tpt_base_input(&record));
        let (patch, fields) = merge(&draft, &stored, base.as_ref(), body.overwrite);
        if fields.is_empty() {
            rows.push(TemplateApplyRow {
                product,
                title,
                verdict: TemplateApplyVerdict::Unchanged,
                fields: Vec::new(),
                reason: None,
            });
            counts.unchanged = counts.unchanged.saturating_add(1);
            continue;
        }
        // The same validation the edit route runs, on the same values, before
        // anything is written. A merge that would leave the resource
        // unsubmittable — a price filled in with no tax code stated, say — is
        // the seller's to fix and is told in the create form's own words.
        match prepare_edit(
            state,
            context.org,
            product,
            &stored,
            &patch,
            &bound,
            EditBar::Filled,
        )
        .await
        {
            Ok(prepared) => {
                rows.push(TemplateApplyRow {
                    product,
                    title,
                    verdict: TemplateApplyVerdict::WillChange,
                    fields,
                    reason: None,
                });
                counts.will_change = counts.will_change.saturating_add(1);
                admitted.push(Admitted { product, prepared });
            }
            Err(refusal) => {
                rows.push(blocked(product, title, sentence(&refusal)));
                counts.blocked = counts.blocked.saturating_add(1);
            }
        }
    }

    Ok(Plan {
        view: TemplateApplyPlanView { rows, counts },
        admitted,
    })
}

/// A refusal as one sentence on a row.
///
/// The create form's own refusals travel in the detail rather than in the
/// message — the message says only that the form's rules were not satisfied,
/// which on a per-resource row would tell the seller nothing they can act on —
/// so those are what this reads, joined. A refusal with no such detail falls
/// back to its message, which is already a sentence.
fn sentence(refusal: &APIError) -> String {
    let mut said: Vec<String> = Vec::new();
    for entry in &refusal.errors {
        let refusals = entry
            .detail
            .as_ref()
            .and_then(|detail| detail.get("refusals"))
            .and_then(|refusals| serde_json::from_value::<Vec<RefusalView>>(refusals.clone()).ok())
            .unwrap_or_default();
        if refusals.is_empty() {
            said.push(entry.message.clone());
        } else {
            said.extend(refusals.into_iter().map(|view| view.message));
        }
    }
    if said.is_empty() {
        "this resource cannot take it".to_owned()
    } else {
        said.join(" ")
    }
}

fn blocked(product: ProductId, title: String, reason: String) -> TemplateApplyRow {
    TemplateApplyRow {
        product,
        title,
        verdict: TemplateApplyVerdict::Blocked,
        fields: Vec::new(),
        reason: Some(reason),
    }
}

/// The seller's tick list, or a collection's membership, resolved against the
/// catalogue.
///
/// A named product the catalogue does not hold is dropped rather than
/// refused, which is `ImportRunRepo::select`'s rule: a stale tab naming a
/// resource that has since been deleted finishes the rest instead of refusing
/// the whole list.
async fn chosen_products(
    state: &AppState,
    org: OrgId,
    selection: &ApplySelection,
) -> Result<Vec<ProductId>, APIError> {
    match selection {
        ApplySelection::Collection { collection } => {
            Ok(ResourceCollectionRepo::new(state.pool.clone())
                .members(org, *collection)
                .await
                .map_err(|error| storage_fault(state, &error))?
                .into_iter()
                .map(|member| member.product)
                .collect())
        }
        ApplySelection::Products { products } => {
            let all = ProductRepo::new(state.pool.clone())
                .list(org)
                .await
                .map_err(|error| storage_fault(state, &error))?;
            Ok(all
                .into_iter()
                .filter(|summary| products.contains(&summary.id))
                .map(|summary| summary.id)
                .collect())
        }
    }
}

// ---------------------------------------------------------------- the merge

/// Fills the fields this template answers and the resource does not, and says
/// which.
///
/// The title is absent on purpose and under `overwrite` too: it is the
/// resource's identity, the one field a template cannot know, and a bulk verb
/// that renamed forty resources to one name would be a data loss with a
/// preview in front of it.
///
/// Every field is listed only where the value it would write differs from the
/// value stored, which is what makes a replay change nothing. So `overwrite`
/// is not "write everything" but "consider a field the resource has already
/// answered", and applying the same template twice under it is still one
/// write.
///
/// Which fields belong to which row: the description, the price and the grades
/// are the catalogue product's own columns, and everything else the create
/// form holds lives in the TPT sidecar — the subject areas, the tags, the
/// formats, the custom categories, the tax code, the copyright attestation,
/// the standards, the details and the localisation. The sidecar is given whole
/// or not at all, because that is `PATCH`'s contract for it, so the merged
/// block is the stored one with the template's fills applied.
fn merge(
    draft: &DraftInput,
    stored: &CanonicalProduct,
    base: Option<&TptBaseInput>,
    overwrite: bool,
) -> (PatchProductBody, Vec<String>) {
    let mut fields: Vec<String> = Vec::new();
    let mut patch = PatchProductBody::default();

    let described = draft.description.trim();
    if !described.is_empty()
        && (overwrite || stored.body.body.trim().is_empty())
        && described != stored.body.body
    {
        patch.body = Some(described.to_owned());
        // Given with the body it describes, which is what the edit route
        // requires; the format is the stored one, because a template holds no
        // opinion about markup and changing it silently is how a listing
        // acquires escaped markdown.
        patch.body_format = Some(stored.body.format);
        fields.push("description".to_owned());
    }

    // A template's price is the create form's own field, so it is denominated
    // the way that form denominates it. Nothing here converts: a resource
    // priced by a template is priced in the template's currency, which is the
    // same statement `resolve_price` makes at the marketplace boundary.
    let priced = if draft.free {
        Some(PriceIntent::Free)
    } else {
        draft
            .price_minor_units
            .and_then(|minor| Money::new(minor, Currency::Usd).ok())
            .map(PriceIntent::Paid)
    };
    if let Some(intent) = priced {
        if (overwrite || matches!(stored.price, PriceIntent::Free)) && intent != stored.price {
            patch.price = Some(intent);
            fields.push("price".to_owned());
        }
    }

    if !draft.grades.is_empty() {
        let held: Vec<String> = stored.grades.raw.iter().map(stored_grade_slug).collect();
        if (overwrite || held.is_empty()) && held != draft.grades {
            patch.grades = Some(
                draft
                    .grades
                    .iter()
                    .map(String::as_str)
                    .map(grade_path)
                    .collect(),
            );
            fields.push("grades".to_owned());
        }
    }

    let sidecar_from = fields.len();
    let mut merged = base.cloned().unwrap_or_default();
    fill_list(
        &mut merged.subject_areas,
        &draft.subject_areas,
        overwrite,
        "subject_areas",
        &mut fields,
    );
    fill_list(
        &mut merged.tags,
        &draft.tags,
        overwrite,
        "tags",
        &mut fields,
    );
    fill_list(
        &mut merged.formats,
        &draft.formats,
        overwrite,
        "formats",
        &mut fields,
    );
    fill_list(
        &mut merged.custom_categories,
        &draft.custom_categories,
        overwrite,
        "custom_categories",
        &mut fields,
    );
    fill(
        &mut merged.tax_code_id,
        narrow(draft.tax_code_id),
        overwrite,
        "tax_code_id",
        &mut fields,
    );
    fill(
        &mut merged.copyright_declaration_id,
        narrow(draft.copyright_declaration_id),
        overwrite,
        "copyright_declaration_id",
        &mut fields,
    );
    fill(
        &mut merged.teaching_duration_id,
        narrow(draft.teaching_duration_id),
        overwrite,
        "teaching_duration_id",
        &mut fields,
    );
    fill(
        &mut merged.answer_key_id,
        narrow(draft.answer_key_id),
        overwrite,
        "answer_key_id",
        &mut fields,
    );
    fill(
        &mut merged.status_user,
        narrow(draft.status_user),
        overwrite,
        "status_user",
        &mut fields,
    );
    fill(
        &mut merged.pages_or_slides,
        draft.pages_or_slides,
        overwrite,
        "pages_or_slides",
        &mut fields,
    );
    fill(
        &mut merged.additional_licence_minor_units,
        draft.additional_licence_minor_units,
        overwrite,
        "additional_licence_minor_units",
        &mut fields,
    );
    fill(
        &mut merged.bundle_discount_minor_units,
        draft.bundle_discount_minor_units,
        overwrite,
        "bundle_discount_minor_units",
        &mut fields,
    );
    fill(
        &mut merged.appropriate_for_country,
        draft.appropriate_for_country,
        overwrite,
        "appropriate_for_country",
        &mut fields,
    );
    fill(
        &mut merged.video_preview_hash,
        draft.video_preview_hash.clone(),
        overwrite,
        "video_preview_hash",
        &mut fields,
    );
    fill(
        &mut merged.thumbnail_mode,
        narrow(draft.thumbnail_mode),
        overwrite,
        "thumbnail_mode",
        &mut fields,
    );
    fill_list(
        &mut merged.thumbnail_hashes,
        &draft.thumbnail_hashes,
        overwrite,
        "thumbnail_hashes",
        &mut fields,
    );
    if !draft.standards.is_empty()
        && (overwrite || merged.standards.is_empty())
        && alignments(&merged.standards) != alignments(&draft.standards)
    {
        merged.standards.clone_from(&draft.standards);
        fields.push("standards".to_owned());
    }
    // The block travels only where the template fills something in it: a
    // resource with no sidecar row is one authored through a path that never
    // held this form, and writing an empty block for it would invent a row
    // the seller never filled.
    if fields.len() > sidecar_from {
        patch.tpt_base = Some(merged);
    }

    (patch, fields)
}

/// One optional field filled where the template answers it and the resource
/// does not, or where the seller asked for the template to win.
fn fill<T: PartialEq>(
    target: &mut Option<T>,
    from: Option<T>,
    overwrite: bool,
    name: &str,
    fields: &mut Vec<String>,
) {
    let Some(value) = from else {
        return;
    };
    if target.is_some() && !overwrite {
        return;
    }
    if target.as_ref() == Some(&value) {
        return;
    }
    *target = Some(value);
    fields.push(name.to_owned());
}

/// The same rule for a picker, where "unanswered" is an empty list rather than
/// an absent value.
fn fill_list(
    target: &mut Vec<String>,
    from: &[String],
    overwrite: bool,
    name: &str,
    fields: &mut Vec<String>,
) {
    if from.is_empty() {
        return;
    }
    if !target.is_empty() && !overwrite {
        return;
    }
    if target.as_slice() == from {
        return;
    }
    *target = from.to_vec();
    fields.push(name.to_owned());
}

/// A wire id as the sidecar's write body holds it, dropping one no byte can
/// hold.
///
/// The draft reads every vocabulary id as `i64` because a form can hold any
/// integer, and the create form refuses an out-of-range one by its control's
/// name. A template carrying one was refused at the template route, so this
/// narrowing drops nothing a stored template holds; it is total here rather
/// than a second refusal, because a template that somehow held one would
/// otherwise block every row of an apply for a field the seller cannot see.
fn narrow(id: Option<i64>) -> Option<u8> {
    id.and_then(|value| u8::try_from(value).ok())
}

/// One grade slug as the catalogue stores it, which is `gradePathsOf`'s own
/// shape in `web/src/lib/tpt-form.ts` and the inverse of
/// [`stored_grade_slug`].
fn grade_path(slug: &str) -> PathInput {
    PathInput {
        inventory: InventoryId::Tpt,
        kind: TermKind::Phase,
        segments: vec![slug.to_owned()],
        native_id: Some(slug.to_owned()),
    }
}

/// Standards compared by what they say rather than by identity, because
/// `StandardInput` is a deserialisation target and carries no equality.
fn alignments(standards: &[StandardInput]) -> Vec<(u32, &str, Option<u64>)> {
    standards
        .iter()
        .map(|standard| {
            (
                standard.framework,
                standard.code.as_str(),
                standard.tpt_node_id,
            )
        })
        .collect()
}

// ------------------------------------------------------- the scheduler's use

/// Fills what a pull left empty on one resource, from a rule's template.
///
/// The auto-publish rule's own use of the merge above, and the same merge: a
/// freshly pulled resource is bound on its source alone and carries what that
/// shop held, which is never the whole form — no marketplace answers our
/// copyright attestation, our tax code, our formats or our details. So the
/// template fills those, fill-empty and never overwriting, before the publish
/// is seeded.
///
/// Answers whether anything was written. A refusal is the caller's to swallow:
/// a rule that cannot fill one resource must still publish the rest, and the
/// resource goes on unfilled rather than the pass failing.
#[expect(
    clippy::too_many_arguments,
    reason = "the fill names the tenant, the resource, the template's draft, the mappings the \
              edit reaches and the instant; a struct over those five would be this signature \
              with a name"
)]
pub(crate) async fn fill_from_template(
    state: &AppState,
    org: OrgId,
    product: ProductId,
    draft: &DraftInput,
    mappings: &[MappingRecord],
    now: Timestamp,
) -> Result<bool, APIError> {
    let Some(record) = ProductRepo::new(state.pool.clone())
        .get(org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?
    else {
        return Ok(false);
    };
    let stored = record.product;
    let base = TptBaseRepo::new(state.pool.clone())
        .get(org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .map(|held| tpt_base_input(&held));
    let (patch, fields) = merge(draft, &stored, base.as_ref(), false);
    if fields.is_empty() {
        return Ok(false);
    }
    let prepared = prepare_edit(
        state,
        org,
        product,
        &stored,
        &patch,
        mappings,
        EditBar::Filled,
    )
    .await?;
    commit_edit(state, org, product, &prepared, now).await
}

#[cfg(test)]
mod tests {
    use super::{merge, DraftInput, TptBaseInput};
    use tam_domain::{
        CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration, VocabularyId,
        VocabularyPath,
    };
    use tam_types::{
        CopyFormat, Currency, InventoryId, ListingCopy, Money, OrgId, PriceIntent, ProductId,
        TermKind, Title, Uuid,
    };

    fn product(body: &str, price: PriceIntent, grades: Vec<&str>) -> CanonicalProduct {
        CanonicalProduct {
            id: ProductId(Uuid([0x01; 16])),
            org: OrgId(Uuid([0xAA; 16])),
            title: Title("Fixture".to_owned()),
            body: ListingCopy {
                body: body.to_owned(),
                format: CopyFormat::Markdown,
            },
            payload: None,
            cover: None,
            previews: vec![],
            subjects: vec![],
            grades: GradeDeclaration {
                source: DeclarationSource::Seller,
                raw: grades
                    .into_iter()
                    .map(|slug| VocabularyPath {
                        vocabulary: VocabularyId(InventoryId::Tpt, TermKind::Phase),
                        segments: vec![slug.to_owned()],
                        native_id: Some(slug.to_owned()),
                    })
                    .collect(),
                derived: None,
            },
            price,
            rights: RightsDeclaration::Unstated,
            native_residue: vec![],
        }
    }

    fn template() -> DraftInput {
        DraftInput {
            description: "Differentiated three ways.".to_owned(),
            price_minor_units: Some(450),
            copyright_declaration_id: Some(1),
            grades: vec!["1st-grade".to_owned()],
            subject_areas: vec!["phonics".to_owned()],
            ..DraftInput::default()
        }
    }

    fn paid(minor: i64) -> PriceIntent {
        PriceIntent::Paid(Money::new(minor, Currency::Usd).expect("a positive amount"))
    }

    #[test]
    fn an_empty_resource_takes_every_field_the_template_answers() {
        let (patch, fields) = merge(
            &template(),
            &product("", PriceIntent::Free, vec![]),
            None,
            false,
        );
        assert_eq!(
            fields,
            vec![
                "description".to_owned(),
                "price".to_owned(),
                "grades".to_owned(),
                "subject_areas".to_owned(),
                "copyright_declaration_id".to_owned(),
            ],
            "the fields named are the ones the apply will write, in the order the form holds them"
        );
        assert_eq!(
            patch.price,
            Some(paid(450)),
            "a free resource takes the template's price"
        );
        assert_eq!(
            patch.tpt_base.map(|base| base.copyright_declaration_id),
            Some(Some(1)),
            "and the sidecar half travels whole, with the attestation filled in"
        );
    }

    #[test]
    fn a_resource_that_has_answered_keeps_its_own_values() {
        let stored = product("Its own description.", paid(1_200), vec!["5th-grade"]);
        let base = TptBaseInput {
            subject_areas: vec!["algebra".to_owned()],
            copyright_declaration_id: Some(2),
            ..TptBaseInput::default()
        };
        let (patch, fields) = merge(&template(), &stored, Some(&base), false);
        assert!(
            fields.is_empty(),
            "a template fills what is empty and leaves what is answered: {fields:?}"
        );
        assert!(
            patch.tpt_base.is_none() && patch.price.is_none() && patch.body.is_none(),
            "so the patch carries nothing at all"
        );
    }

    #[test]
    fn overwrite_replaces_answered_fields_and_is_still_idempotent() {
        let stored = product("Its own description.", paid(1_200), vec!["5th-grade"]);
        let base = TptBaseInput {
            subject_areas: vec!["algebra".to_owned()],
            copyright_declaration_id: Some(2),
            ..TptBaseInput::default()
        };
        let (_patch, fields) = merge(&template(), &stored, Some(&base), true);
        assert_eq!(
            fields,
            vec![
                "description".to_owned(),
                "price".to_owned(),
                "grades".to_owned(),
                "subject_areas".to_owned(),
                "copyright_declaration_id".to_owned(),
            ],
            "overwrite considers a field the resource has answered"
        );
        // The same template applied to the resource it has already been
        // applied to, which is what a retried request is.
        let settled = product("Differentiated three ways.", paid(450), vec!["1st-grade"]);
        let filled = TptBaseInput {
            subject_areas: vec!["phonics".to_owned()],
            copyright_declaration_id: Some(1),
            ..TptBaseInput::default()
        };
        let (_again, second) = merge(&template(), &settled, Some(&filled), true);
        assert!(
            second.is_empty(),
            "and writes nothing the second time, because a field is listed only where the \
             value differs: {second:?}"
        );
    }

    #[test]
    fn a_title_is_never_filled_from_a_template() {
        let named = DraftInput {
            name: "The template's own title".to_owned(),
            ..template()
        };
        let (patch, _fields) = merge(&named, &product("", PriceIntent::Free, vec![]), None, true);
        assert!(
            patch.title.is_none(),
            "a title is the resource's identity and the one field a template cannot know"
        );
    }

    #[test]
    fn a_free_template_does_not_zero_a_priced_resource_unless_asked() {
        let free = DraftInput {
            free: true,
            price_minor_units: None,
            ..DraftInput::default()
        };
        let stored = product("Its own description.", paid(1_200), vec![]);
        let (patch, fields) = merge(&free, &stored, None, false);
        assert!(
            patch.price.is_none() && fields.is_empty(),
            "the resource has answered the price, so fill-empty leaves it: {fields:?}"
        );
        let (patch, fields) = merge(&free, &stored, None, true);
        assert_eq!(
            (patch.price, fields),
            (Some(PriceIntent::Free), vec!["price".to_owned()]),
            "and overwrite is how a seller states they meant it"
        );
    }
}
