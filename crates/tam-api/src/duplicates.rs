//! The duplicate review: one card per pair, one sentence of evidence, three
//! buttons, and thirty days to change your mind.
//!
//! What the seller is asked is deliberately narrow. The matcher has already
//! thrown away everything below the review floor and merged everything on a
//! decisive layer, so a pair reaching here is one the machinery could not
//! decide: two independent moderate signals, which is exactly the band where
//! a fused web-page matcher's best published precision was 0.79. One wrong
//! merge in five is unacceptable when the next publish would overwrite a live
//! listing, and one review click is not.
//!
//! Three answers, and the third is an answer. `Same` merges; `Different` is
//! stored, because without it the next import asks again and the seller
//! learns that answering achieves nothing; `Decide later` parks the pair and
//! never blocks the import it came from.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    DuplicateRepo, FingerprintRepo, ImportRunRepo, ProductEdit, ProductRepo, Verdict,
};
use tam_types::{InventoryId, OrgId, ProductId, Timestamp, Uuid};

use crate::entitlement::feature_refusal;
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::import_runs::{item_cover_url, MatchLayerView, ReviewPairView, ReviewSideView};
use crate::jobs::{missing, storage_fault, validation};
use crate::{AppState, OrgContext};

/// Every open pair, or the open pairs of one run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicatesView {
    pub pairs: Vec<ReviewPairView>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DuplicatesQuery {
    pub run: Option<String>,
}

/// Which side's value survives, per field.
///
/// Only the three fields a seller can tell apart at a glance on the card. The
/// taxonomy, the grades and the files are not offered: they are not rendered
/// side by side, and a which-side-wins control over a value nobody can see is
/// a coin toss wearing a choice's clothes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WinningSide {
    Lo,
    Hi,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WinningFields {
    pub title: Option<WinningSide>,
    pub description: Option<WinningSide>,
    pub price: Option<WinningSide>,
}

/// The seller's answer.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum VerdictBody {
    Same {
        /// Which product survives. One of the pair, checked: a survivor
        /// outside the pair would tombstone both sides and keep neither.
        keep: Uuid,
        #[serde(default)]
        fields: WinningFields,
    },
    Different,
    Parked,
}

/// What a verdict did.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerdictAck {
    pub product_lo: ProductId,
    pub product_hi: ProductId,
    pub verdict: DuplicateVerdictView,
    /// The survivor, on a merge.
    pub kept: Option<ProductId>,
    /// When the merge stops being reversible, on a merge.
    pub reversible_until: Option<Timestamp>,
}

/// What was decided about a pair, on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateVerdictView {
    Same,
    Different,
    Parked,
}

impl DuplicateVerdictView {
    /// The closed set, in a stable order, for the vocabulary generator.
    pub const ALL: [Self; 3] = [Self::Same, Self::Different, Self::Parked];

    #[must_use]
    pub const fn of(verdict: Verdict) -> Self {
        match verdict {
            Verdict::Same => Self::Same,
            Verdict::Different => Self::Different,
            Verdict::Parked => Self::Parked,
        }
    }
}

// ---------------------------------------------------------------- handlers

pub(crate) async fn list_duplicates(
    State(state): State<AppState>,
    context: OrgContext,
    Query(query): Query<DuplicatesQuery>,
) -> Result<Json<DuplicatesView>, APIError> {
    let run = query.run.as_deref().map(parse_id).transpose()?;
    Ok(Json(DuplicatesView {
        pairs: pairs_of(&state, context.org, run).await?,
    }))
}

/// Records the seller's answer about one pair.
///
/// The answer and everything it does — the merge, the tombstone, the import
/// row returning to the commit queue — are written in one organisation-scoped
/// decision transaction, the same one the import's own commit takes. That is
/// what orders a verdict against a commit rather than letting the two
/// interleave: a cancellation that committed first rejects the commit this
/// answer would have unblocked, and a commit that ran first stays visible.
pub(crate) async fn decide(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, lo, hi)): Path<(String, String, String)>,
    Json(body): Json<VerdictBody>,
) -> Result<Json<VerdictAck>, APIError> {
    if !context.entitlement.caps.duplicate_review {
        return Err(feature_refusal(
            "duplicate_review",
            "Your plan does not include the duplicate review. Nothing is blocked: your import \
             brings everything in and nothing is merged.",
        ));
    }
    let (lo, hi) = tam_storage::ordered_pair(ProductId(parse_id(&lo)?), ProductId(parse_id(&hi)?));
    let now = (state.wall)();
    // The survivor the seller named, checked before anything else: a survivor
    // outside the pair would tombstone both sides and keep neither.
    let keep = match &body {
        VerdictBody::Same { keep, .. } => {
            let keep = ProductId(*keep);
            if keep != lo && keep != hi {
                return Err(validation(
                    "the resource you keep has to be one of the two this question is about",
                ));
            }
            Some(keep)
        }
        VerdictBody::Different | VerdictBody::Parked => None,
    };

    let mut tx = crate::import_runs::begin_guarded(&state, context.org).await?;
    // The pair itself, re-read under the lock that is about to act on it.
    //
    // Not tidiness: two answers to one card can both pass an unlocked
    // eligibility check, and then the first tombstones B while the second
    // tombstones A and overwrites the verdict to keep the product it has just
    // deleted. The row is taken `FOR UPDATE` here, so the second answer finds
    // the question answered and says so.
    let standing = tam_storage::pair_verdict(&mut tx, context.org, lo, hi)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let Some((Verdict::Parked, _, _)) = standing else {
        tx.rollback().await.map_err(|error| {
            state.internal(&format!("the database refused a transaction: {error}"))
        })?;
        return Err(match standing {
            Some(_) => APIError::new(
                StatusCode::CONFLICT,
                APIErrorEntry::new("this pair has been answered already")
                    .code(APIErrorCode::DuplicatePairSettled)
                    .kind(APIErrorKind::Validation),
            ),
            None => missing("no such duplicate question"),
        });
    };

    // Each side that is an import row is decided under its own run's guard: a
    // run the seller cancelled does not advance items, however the question
    // about them is answered. The verdict itself is still recorded — the
    // seller answered it, and the answer is theirs to keep.
    let (verdict, kept) = match &body {
        VerdictBody::Same { fields, .. } => {
            let keep = keep.ok_or_else(|| state.internal("a merge names no survivor"))?;
            let loser = if keep == lo { hi } else { lo };
            merge(
                &state,
                &mut tx,
                context.org,
                &Merging {
                    keep,
                    loser,
                    lo,
                    hi,
                    fields,
                },
                now,
            )
            .await?;
            (Verdict::Same, Some(keep))
        }
        VerdictBody::Different => (Verdict::Different, None),
        VerdictBody::Parked => (Verdict::Parked, None),
    };
    tam_storage::decide_verdict(&mut tx, context.org, lo, hi, verdict, kept, now)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    // The answer is what unblocks the item. An import run item is held in
    // review by the pairs it is in rather than by a flag, so answering the
    // last of them has to move it -- otherwise a seller who said "different"
    // would watch a commit that never reaches the resource they just said to
    // keep.
    unblock(&state, &mut tx, context.org, lo).await?;
    unblock(&state, &mut tx, context.org, hi).await?;
    tx.commit()
        .await
        .map_err(|error| state.internal(&format!("the database refused a transaction: {error}")))?;
    Ok(Json(VerdictAck {
        product_lo: lo,
        product_hi: hi,
        verdict: DuplicateVerdictView::of(verdict),
        kept,
        reversible_until: (verdict == Verdict::Same)
            .then(|| Timestamp(now.0.saturating_add(tam_storage::REVERSIBLE_MS))),
    }))
}

/// Reverses a merge, inside the window the seller was told.
///
/// A compensation rather than a reopening: the historical run stays settled
/// and keeps what it recorded, and what comes back is the resource — the
/// tombstoned product restored, or the import row returned to its run's
/// commit queue if that run still permits work.
pub(crate) async fn undo(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, lo, hi)): Path<(String, String, String)>,
) -> Result<Json<VerdictAck>, APIError> {
    if !context.entitlement.caps.duplicate_review {
        return Err(feature_refusal(
            "duplicate_review",
            "Your plan does not include the duplicate review.",
        ));
    }
    let (lo, hi) = tam_storage::ordered_pair(ProductId(parse_id(&lo)?), ProductId(parse_id(&hi)?));
    let now = (state.wall)();

    let mut tx = crate::import_runs::begin_guarded(&state, context.org).await?;
    // The merge this reverses, re-read under the lock that is about to undo
    // it: two undos racing, or an undo racing a re-decision, otherwise both
    // pass an unlocked eligibility check and the second acts on a merge that
    // no longer stands.
    let standing = tam_storage::pair_verdict(&mut tx, context.org, lo, hi)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let Some((Verdict::Same, Some(kept), Some(until))) = standing else {
        tx.rollback().await.map_err(|error| {
            state.internal(&format!("the database refused a transaction: {error}"))
        })?;
        return Err(match standing {
            Some(_) => validation("only a merge can be undone, and this pair was not merged"),
            None => missing("no such duplicate question"),
        });
    };
    if now.0 > until.0 {
        tx.rollback().await.map_err(|error| {
            state.internal(&format!("the database refused a transaction: {error}"))
        })?;
        return Err(validation(
            "the thirty days to undo this merge have passed; the two are one resource now",
        ));
    }
    let loser = if kept == lo { hi } else { lo };

    // Either the loser was a product this merge tombstoned, or it was a run
    // item this merge skipped. Both are reversed in this transaction, and
    // neither is a special case of the other: the run item's reversal is what
    // puts the resource back in the import's own queue.
    let restored = tam_storage::restore_product(&mut tx, context.org, loser, now)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if restored {
        if let Some(marketplace) = marketplace_of(&state, context.org, loser).await? {
            tam_storage::attach_system_label(&mut tx, context.org, loser, marketplace, now)
                .await
                .map_err(|error| storage_fault(&state, &error))?;
        }
    } else if let Some((run, locator)) = tam_storage::item_of_product(&mut tx, context.org, loser)
        .await
        .map_err(|error| storage_fault(&state, &error))?
    {
        // Only while that run still permits work. A completed or cancelled
        // run is not reopened by an undo — the compensation is audited against
        // the catalogue, not against a historical run's state.
        if advanceable(&state, &mut tx, context.org, run).await? {
            let at = tam_storage::ItemAddress {
                run,
                locator: &locator,
            };
            // A merged item is `skipped` and settled, which the ordinary
            // verdict write refuses to move. Reopening it is the undo's own
            // transition: without it the pair came back as a question while
            // the resource stayed skipped, and no answer the seller then gave
            // could ever commit it.
            if !tam_storage::reopen_skipped(&mut tx, context.org, at)
                .await
                .map_err(|error| storage_fault(&state, &error))?
            {
                tam_storage::record_verdict(
                    &mut tx,
                    context.org,
                    at,
                    tam_storage::RunItemState::Review,
                )
                .await
                .map_err(|error| storage_fault(&state, &error))?;
            }
        }
    }

    tam_storage::decide_verdict(&mut tx, context.org, lo, hi, Verdict::Parked, None, now)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    tx.commit()
        .await
        .map_err(|error| state.internal(&format!("the database refused a transaction: {error}")))?;
    Ok(Json(VerdictAck {
        product_lo: lo,
        product_hi: hi,
        verdict: DuplicateVerdictView::Parked,
        kept: None,
        reversible_until: None,
    }))
}

/// Whether this run still lets its items move.
///
/// The run guard, read under the same lock as the write it gates: an open run
/// advances, and a settled one — completed, failed or stopped — does not.
async fn advanceable(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    run: Uuid,
) -> Result<bool, APIError> {
    Ok(tam_storage::guard_run(tx, org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .is_some_and(|guard| guard.state.open()))
}

/// Returns one side to the commit queue if nothing is still owed about it.
///
/// Only where that side is an import run item still in review, and only while
/// its run's guard permits work: a product needs no unblocking, and an item
/// the seller merged away is settled rather than waiting.
async fn unblock(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    product: ProductId,
) -> Result<(), APIError> {
    let Some((run, locator)) = tam_storage::item_of_product(tx, org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?
    else {
        return Ok(());
    };
    let at = tam_storage::ItemAddress {
        run,
        locator: &locator,
    };
    if tam_storage::reserved_state(tx, org, at)
        .await
        .map_err(|error| storage_fault(state, &error))?
        != Some(tam_storage::RunItemState::Review)
    {
        return Ok(());
    }
    if tam_storage::parked_for(tx, org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?
        > 0
    {
        return Ok(());
    }
    if !advanceable(state, tx, org, run).await? {
        return Ok(());
    }

    tam_storage::record_verdict(tx, org, at, tam_storage::RunItemState::Matched)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(())
}

// ------------------------------------------------------------------ merge

/// One merge, as the seller answered it.
struct Merging<'a> {
    keep: ProductId,
    loser: ProductId,
    lo: ProductId,
    hi: ProductId,
    fields: &'a WinningFields,
}

/// Applies a `same` verdict: the survivor keeps the fields the seller chose,
/// and the loser stops being a second resource.
///
/// Every write is on the caller's transaction, so the survivor's fields, the
/// loser's tombstone, the label that moves and the verdict recording the
/// decision either all stand or none do. A process that stopped between them
/// used to leave a tombstoned resource beside a question that still read as
/// unanswered, with no recorded merge to reverse.
async fn merge(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    merging: &Merging<'_>,
    now: Timestamp,
) -> Result<(), APIError> {
    let products = ProductRepo::new(state.pool.clone());
    let held_lo = products
        .get(org, merging.lo)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let held_hi = products
        .get(org, merging.hi)
        .await
        .map_err(|error| storage_fault(state, &error))?;

    // The which-side-wins step, applied only where both sides exist as
    // products: a field cannot be taken from a resource that was never
    // created, and the run item's values are already the ones its own commit
    // would have written.
    let mut edit = ProductEdit {
        title: None,
        body: None,
        price: None,
        subjects: None,
        grades: None,
        rights: None,
    };
    let pick = |side: WinningSide| match side {
        WinningSide::Lo => held_lo.as_ref(),
        WinningSide::Hi => held_hi.as_ref(),
    };
    if let Some(chosen) = merging.fields.title.and_then(pick) {
        edit.title = Some(chosen.product.title.clone());
    }
    if let Some(chosen) = merging.fields.description.and_then(pick) {
        edit.body = Some(chosen.product.body.clone());
    }
    if let Some(chosen) = merging.fields.price.and_then(pick) {
        edit.price = Some(chosen.product.price);
    }
    if edit.title.is_some() || edit.body.is_some() || edit.price.is_some() {
        tam_storage::update_product(tx, org, merging.keep, &edit, now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
    }

    // The survivor gains the label of the shop the loser came from, because it
    // now stands for that listing too.
    if let Some(marketplace) = marketplace_of(state, org, merging.loser).await? {
        tam_storage::attach_system_label(tx, org, merging.keep, marketplace, now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
    }

    let loser_is_a_product = if merging.keep == merging.lo {
        held_hi.is_some()
    } else {
        held_lo.is_some()
    };
    if loser_is_a_product {
        // Tombstoned rather than erased, which is exactly what makes the
        // thirty-day reversal possible.
        tam_storage::soft_delete_product(tx, org, merging.loser, now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        return Ok(());
    }

    // The loser is a resource an import read and has not created. Skipping it
    // is the merge: the survivor already holds this resource, and creating a
    // second one is precisely what the verdict says not to do.
    //
    // Only while that import still permits work, and the guard is read here
    // rather than left to the unblock afterwards: an answer that arrives
    // after the seller stopped the run would otherwise settle a row of a
    // terminal import, which is the one thing the cancellation was supposed
    // to have stopped. The verdict itself is still recorded — the seller
    // answered the question and the answer is theirs to keep.
    if let Some((run, locator)) = tam_storage::item_of_product(tx, org, merging.loser)
        .await
        .map_err(|error| storage_fault(state, &error))?
    {
        if !advanceable(state, tx, org, run).await? {
            return Ok(());
        }
        let title = tam_storage::title_of(tx, org, merging.keep)
            .await
            .map_err(|error| storage_fault(state, &error))?
            .unwrap_or_else(|| "a resource you already have".to_owned());
        tam_storage::record_skipped(
            tx,
            org,
            tam_storage::ItemAddress {
                run,
                locator: &locator,
            },
            &format!("same as {title}"),
            now,
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
    }
    Ok(())
}

// ------------------------------------------------------------------ views

/// The open pairs, as review cards.
pub(crate) async fn pairs_of(
    state: &AppState,
    org: OrgId,
    run: Option<Uuid>,
) -> Result<Vec<ReviewPairView>, APIError> {
    let held = DuplicateRepo::new(state.pool.clone())
        .open_pairs(org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let mut pairs = Vec::with_capacity(held.len());
    for record in &held {
        pairs.push(ReviewPairView {
            product_lo: record.lo,
            product_hi: record.hi,
            sentence: crate::matcher::sentence_for(record.winning_layer, &record.evidence),
            layer: MatchLayerView::of(record.winning_layer),
            lo: side_of(state, org, record.lo).await?,
            hi: side_of(state, org, record.hi).await?,
        });
    }
    Ok(pairs)
}

/// One side of a card: whichever of the two things that identifier names.
async fn side_of(
    state: &AppState,
    org: OrgId,
    product: ProductId,
) -> Result<ReviewSideView, APIError> {
    if let Some(held) = ProductRepo::new(state.pool.clone())
        .get(org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?
    {
        let meta = FingerprintRepo::new(state.pool.clone())
            .metadata_for(org, &[product])
            .await
            .map_err(|error| storage_fault(state, &error))?;
        let meta = meta.first();
        return Ok(ReviewSideView {
            product_id: Some(product),
            run_locator: None,
            marketplace: meta.and_then(|meta| meta.marketplaces.first().copied()),
            title: held.product.title.0.clone(),
            price: match held.product.price {
                tam_types::PriceIntent::Paid(money) => Some(money),
                tam_types::PriceIntent::Free => None,
            },
            grades: meta
                .and_then(|meta| grade_phrase(meta.grade_low, meta.grade_high))
                .into_iter()
                .collect(),
            cover_url: held
                .product
                .cover
                .as_ref()
                .map(|_| format!("/v1/products/{}/cover", uuid_text(product.0))),
        });
    }

    let runs = ImportRunRepo::new(state.pool.clone());
    let Some((run, locator)) = runs
        .item_by_product(org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?
    else {
        // Neither a product nor a run item. The pair outlived both sides,
        // which a stored `different` answer is allowed to do; the card names
        // it rather than dropping the row, because a question with a missing
        // side is something the seller should see rather than a silent gap.
        return Ok(ReviewSideView {
            product_id: None,
            run_locator: None,
            marketplace: None,
            title: "a resource you no longer have".to_owned(),
            price: None,
            grades: Vec::new(),
            cover_url: None,
        });
    };
    let item = runs
        .item(org, run, &locator)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let head = runs
        .get(org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .map(|record| record.head);
    Ok(ReviewSideView {
        product_id: None,
        run_locator: Some(locator.clone()),
        marketplace: head
            .as_ref()
            .and_then(|head| head.source)
            .map(InventoryId::marketplace),
        title: item
            .as_ref()
            .and_then(|item| item.title.clone())
            .unwrap_or_else(|| locator.clone()),
        price: item.as_ref().and_then(|item| item.price),
        grades: Vec::new(),
        cover_url: item
            .as_ref()
            .and_then(|item| item.cover_hash.map(|_| item_cover_url(run, item.ordinal))),
    })
}

/// The grade band as a phrase, where the read derived one.
fn grade_phrase(low: Option<i16>, high: Option<i16>) -> Option<String> {
    let (low, high) = (low?, high?);
    Some(format!("Ages {low}\u{2013}{high}"))
}

/// Which marketplace a product's listings are on, for the label a merge moves.
async fn marketplace_of(
    state: &AppState,
    org: OrgId,
    product: ProductId,
) -> Result<Option<tam_types::Marketplace>, APIError> {
    let meta = FingerprintRepo::new(state.pool.clone())
        .metadata_for(org, &[product])
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(meta
        .first()
        .and_then(|meta| meta.marketplaces.first().copied()))
}

fn parse_id(raw: &str) -> Result<Uuid, APIError> {
    uuid::Uuid::parse_str(raw)
        .map(|id| Uuid(*id.as_bytes()))
        .map_err(|_| missing("no such duplicate question"))
}

fn uuid_text(id: Uuid) -> String {
    uuid::Uuid::from_bytes(id.0).to_string()
}
