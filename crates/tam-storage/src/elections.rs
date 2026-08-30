//! The durable half of the election queue: the per-product questions the
//! projection could not settle, and the standing rules that stop them being
//! asked twice.
//!
//! Modelled on `TaxonomyRepo` and deliberately not merged into it. The
//! reconciliation queue drains because its answer is a fact about a vocabulary
//! pair that every later product reuses; an election's answer is a fact about
//! one product against one target, so its reuse mechanism is a rule the seller
//! states once rather than an index that collapses rows.
//!
//! Nothing here stores a candidate list or a suggestion. Candidates are
//! derived at read time from the edge relation, because a stored list is a
//! snapshot that goes stale the moment a vocabulary is re-polled; and a
//! suggestion is computed at exactly one place — the read — which is what
//! makes "we only compute a best fit where the seller asked us to" a property
//! of one function rather than a convention scattered across writers.

use sqlx::PgPool;
use tam_domain::equivalence::{
    Election, ElectionAnswer, ElectionRule, ElectionTriggerKind, NewElectionRule, SettledElection,
};
use tam_domain::{TermKind, VocabularyId, VocabularyPath};
use tam_types::{InventoryId, MappingId, OrgId, ProductId, Timestamp, Uuid};

use crate::codec::{
    decider_from_db, decider_to_db, inventory_from_db, inventory_to_db, term_kind_from_db,
    term_kind_to_db, timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db,
};
use crate::jobs::{revive_on, ELECTION};
use crate::taxonomy::RaiseReport;
use crate::{pin_org, StorageError};

pub struct ElectionRepo {
    pool: PgPool,
}

/// One open question as stored. The candidates a seller chooses among are not
/// here: they are read out of the relation when the decision surface is
/// assembled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenElection {
    pub id: Uuid,
    pub product: ProductId,
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub trigger_kind: ElectionTriggerKind,
    pub trigger_key: Option<String>,
    pub raised_by: Option<MappingId>,
    pub raised_at: Timestamp,
}

/// What answering one election did, including how many parked items the answer
/// released. The count is returned rather than logged because an answer that
/// releases nothing is the shape a silently-unpinned update takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnswerReport {
    pub revived: u64,
    /// Whether the answer also became the tenant's standing rule. Reported
    /// rather than assumed, because a surface that offered "apply to future"
    /// and wrote nothing would be telling the seller they had answered once
    /// for good when they had answered once.
    pub promoted: bool,
}

/// One answer as the decision surface submits it: which question, the values
/// the seller named, when, and the standing rule to write beside it where they
/// said to apply it to everything after this.
///
/// Bundled because the arity would otherwise exceed the workspace argument
/// limit, and because a promotion written in a second transaction is a
/// promotion a crash can lose while the answer survives.
pub struct NewAnswer<'a> {
    pub item: Uuid,
    pub paths: &'a [VocabularyPath],
    pub at: Timestamp,
    /// The rule to upsert in the same transaction that records the answer.
    /// `None` keeps the decision to this one product, which is the default:
    /// promotion is explicit, never inferred from the answer's shape.
    ///
    /// It arrives already constructed for the same reason `upsert_rule`'s does
    /// — `ElectionRule::new` is the domain half of the legal backstop and this
    /// must not be a second way in past it.
    pub promote: Option<&'a ElectionRule>,
}

/// One question answered on the create form, before any mapping exists.
///
/// A separate type from [`NewAnswer`], which settles an open row the
/// projection raised: this one mints the row and the answer together, so it
/// names the question rather than an item identifier.
pub struct AnsweredElection<'a> {
    pub product: ProductId,
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub trigger_kind: ElectionTriggerKind,
    /// `free` or `paid` for a supply, the source value's own native id for a
    /// narrow, and `None` for the two kinds that generalise to nothing.
    pub trigger_key: Option<&'a str>,
    pub paths: &'a [VocabularyPath],
}

impl ElectionRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Raises one item per election, deduplicated against the open queue by
    /// the partial unique index and against the settled queue by the answered
    /// row itself.
    ///
    /// The rule check happens before this and is pure, so the common path
    /// writes nothing at all; what reaches here is what no standing answer
    /// covers. The answered-row guard is the fence behind that: the partial
    /// index spans open rows alone, so without it a question the seller has
    /// already answered would mint a second open row on every re-projection
    /// and the queue would grow one duplicate per pass. It is keyed on the
    /// question rather than on the axis, so a price flip still asks the
    /// paid-branch question the free-branch answer says nothing about.
    ///
    /// `already_open` therefore counts every election this raise minted no
    /// row for, whether one was open or the seller had already settled it.
    pub async fn raise(
        &self,
        org: OrgId,
        raised_by: MappingId,
        elections: &[Election],
        at: Timestamp,
    ) -> Result<RaiseReport, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let mut report = RaiseReport {
            new: 0,
            already_open: 0,
        };
        for election in elections {
            let inserted = sqlx::query!(
                "INSERT INTO election_item \
                 (org_id, id, product_id, inventory, axis, trigger_kind, trigger_key, \
                  raised_by, raised_at, state) \
                 SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, 'open' \
                 WHERE NOT EXISTS ( \
                     SELECT 1 FROM election_item \
                     WHERE org_id = $1 AND product_id = $3 AND inventory = $4 \
                       AND axis = $5 AND trigger_kind = $6 AND trigger_key = $7 \
                       AND state = 'answered') \
                 ON CONFLICT DO NOTHING",
                uuid_to_db(org.0),
                uuid::Uuid::new_v4(),
                uuid_to_db(election.product.0),
                inventory_to_db(election.inventory),
                term_kind_to_db(election.axis),
                election.trigger.kind().as_str(),
                election.trigger.key().unwrap_or_default(),
                uuid_to_db(raised_by.0),
                timestamp_to_db(at)?,
            )
            .execute(&mut *tx)
            .await?
            .rows_affected();
            report.new += inserted;
            report.already_open += 1 - inserted;
        }
        tx.commit().await?;
        Ok(report)
    }

    /// Records a question the seller answered before it was ever asked.
    ///
    /// The create form is the one place an answer can precede every mapping:
    /// the seller picks a licence while authoring, and the projection that
    /// would otherwise raise the question runs for the first time after the
    /// product exists. `election_item.raised_by` is nullable for exactly this
    /// shape and migration 0023's provenance CHECK admits it
    /// (`raised_by IS NOT NULL OR state = 'answered'`), so nothing here
    /// invents a mapping to name.
    ///
    /// Nothing is revived, because nothing can be parked on a question no
    /// mapping has raised yet.
    ///
    /// Guarded against a duplicate the way `raise` is: an identical question
    /// this tenant has already settled for this product writes no second row,
    /// so a retried create is a no-op rather than a second answer.
    /// `false` reports exactly that.
    pub async fn record_answered(
        &self,
        org: OrgId,
        answered: &AnsweredElection<'_>,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        if answered.paths.is_empty() {
            return Err(StorageError::Inconsistent {
                reason: "an answered election names at least one value".to_owned(),
            });
        }
        // The CHECK ties the sentinel to the trigger kind, so a violation here
        // would surface as a bare constraint failure the caller cannot read.
        let keyed = matches!(
            answered.trigger_kind,
            ElectionTriggerKind::Supply | ElectionTriggerKind::Narrow
        );
        let key = answered.trigger_key.unwrap_or_default();
        if keyed == key.is_empty() {
            return Err(StorageError::Inconsistent {
                reason: format!(
                    "a {} election carries {} trigger key",
                    answered.trigger_kind.as_str(),
                    if keyed { "no" } else { "a" }
                ),
            });
        }
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = sqlx::query!(
            "INSERT INTO election_item \
             (org_id, id, product_id, inventory, axis, trigger_kind, trigger_key, \
              raised_by, raised_at, state, answer, resolved_at) \
             SELECT $1, $2, $3, $4, $5, $6, $7, NULL, $8, 'answered', $9, $8 \
             WHERE NOT EXISTS ( \
                 SELECT 1 FROM election_item \
                 WHERE org_id = $1 AND product_id = $3 AND inventory = $4 \
                   AND axis = $5 AND trigger_kind = $6 AND trigger_key = $7 \
                   AND state = 'answered') \
             ON CONFLICT DO NOTHING",
            uuid_to_db(org.0),
            uuid::Uuid::new_v4(),
            uuid_to_db(answered.product.0),
            inventory_to_db(answered.inventory),
            term_kind_to_db(answered.axis),
            answered.trigger_kind.as_str(),
            key,
            timestamp_to_db(at)?,
            encode_paths(answered.paths),
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(written == 1)
    }

    /// Every open question for one tenant, in a deterministic order.
    pub async fn open_items(&self, org: OrgId) -> Result<Vec<OpenElection>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT id, product_id, inventory, axis, trigger_kind, trigger_key, \
                    raised_by, raised_at \
             FROM election_item WHERE org_id = $1 AND state = 'open' \
             ORDER BY raised_at, id",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(OpenElection {
                    id: uuid_from_db(row.id),
                    product: ProductId(uuid_from_db(row.product_id)),
                    inventory: inventory_from_db(&row.inventory)?,
                    axis: term_kind_from_db(&row.axis)?,
                    trigger_kind: trigger_kind_from_db(&row.trigger_kind)?,
                    trigger_key: (!row.trigger_key.is_empty()).then_some(row.trigger_key),
                    raised_by: row.raised_by.map(|id| MappingId(uuid_from_db(id))),
                    raised_at: timestamp_from_db(row.raised_at),
                })
            })
            .collect()
    }

    /// Settles one open question and releases whatever it was blocking, in one
    /// transaction.
    ///
    /// The revive is not a follow-up call: a crash between recording the
    /// seller's answer and requeueing the item would leave the item parked for
    /// the full park expiry, which is exactly the latency the answer exists to
    /// remove. `revive_on` pins the tenant itself, because `job_item` carries
    /// FORCE ROW LEVEL SECURITY and an unpinned update returns `Ok(0)`.
    pub async fn answer(
        &self,
        org: OrgId,
        new: NewAnswer<'_>,
    ) -> Result<AnswerReport, StorageError> {
        let NewAnswer {
            item,
            paths,
            at,
            promote,
        } = new;
        if paths.is_empty() {
            return Err(StorageError::Inconsistent {
                reason: "an answered election names at least one value".to_owned(),
            });
        }
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT raised_by FROM election_item \
             WHERE org_id = $1 AND id = $2 AND state = 'open' FOR UPDATE",
            uuid_to_db(org.0),
            uuid_to_db(item),
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StorageError::Inconsistent {
            reason: "the item is not open".to_owned(),
        })?;
        sqlx::query!(
            "UPDATE election_item SET state = 'answered', answer = $3, resolved_at = $4 \
             WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(item),
            encode_paths(paths),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        if let Some(rule) = promote {
            if rule.org != org {
                return Err(StorageError::Inconsistent {
                    reason: "a standing rule belongs to the tenant that answered".to_owned(),
                });
            }
            write_rule(&mut tx, rule).await?;
        }
        let revived = match row.raised_by {
            Some(mapping) => {
                revive_on(&mut tx, org, MappingId(uuid_from_db(mapping)), ELECTION, at).await?
            }
            None => 0,
        };
        tx.commit().await?;
        Ok(AnswerReport {
            revived,
            promoted: promote.is_some(),
        })
    }

    /// Requeues an item parked on the election gate when nothing is left open
    /// to answer.
    ///
    /// The park is not the raise. `prepare_item` commits the raise in its own
    /// transaction and returns `Blocked`; the worker parks the item in a
    /// separate statement afterwards, and in that window the item is still
    /// `leased`. An answer arriving there finds `revive_on`'s
    /// `state = 'parked_live'` predicate matching nothing, reports
    /// `revived: 0`, and the park that follows has nothing left to clear it,
    /// so the seller waits out the full day after deciding.
    ///
    /// Called after the park rather than before it, which is what makes it
    /// total: an answer landing before this read is seen by it, and one
    /// landing after it finds the item parked and revives it itself. Both
    /// orders end with the item queued, and a double revive is a no-op
    /// because the requeue is predicated on the parked state.
    ///
    /// Counted over the mapping's own product and inventory rather than over
    /// `raised_by`, because that is what the projection re-raises: an
    /// election minted under an earlier mapping generation asks the same
    /// question about the same listing.
    pub async fn revive_if_answered(
        &self,
        org: OrgId,
        mapping: MappingId,
        at: Timestamp,
    ) -> Result<u64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let open = sqlx::query_scalar!(
            r#"SELECT count(*) AS "open!" FROM election_item e
               JOIN mapping m
                 ON m.org_id = e.org_id AND m.product_id = e.product_id
                AND m.inventory = e.inventory
               WHERE e.org_id = $1 AND m.id = $2 AND e.state = 'open'"#,
            uuid_to_db(org.0),
            uuid_to_db(mapping.0),
        )
        .fetch_one(&mut *tx)
        .await?;
        if open > 0 {
            tx.commit().await?;
            return Ok(0);
        }
        let revived = revive_on(&mut tx, org, mapping, ELECTION, at).await?;
        tx.commit().await?;
        Ok(revived)
    }

    /// Every question this product's seller has already settled, in the form
    /// `settled_by` reads.
    ///
    /// The projection's other half. `rules` answers what the seller stated as
    /// a policy; this answers what they stated about this product, which is
    /// what an answer on the decision surface is. Without it the revive the
    /// answer performs requeues an item that re-raises the identical question
    /// and parks again, so answering could never release anything.
    pub async fn answered_for(
        &self,
        org: OrgId,
        product: ProductId,
    ) -> Result<Vec<SettledElection>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT inventory, axis, trigger_kind, trigger_key, answer \
             FROM election_item \
             WHERE org_id = $1 AND product_id = $2 AND state = 'answered' \
             ORDER BY resolved_at, id",
            uuid_to_db(org.0),
            uuid_to_db(product.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                let inventory = inventory_from_db(&row.inventory)?;
                let axis = term_kind_from_db(&row.axis)?;
                Ok(SettledElection {
                    product,
                    inventory,
                    axis,
                    trigger_kind: trigger_kind_from_db(&row.trigger_kind)?,
                    trigger_key: (!row.trigger_key.is_empty()).then_some(row.trigger_key),
                    chosen: answered_paths(&row.answer, VocabularyId(inventory, axis))?,
                })
            })
            .collect()
    }

    /// Withdraws a question nobody needs answered any more — the product's
    /// price changed, the mapping was severed — without recording an answer
    /// the seller never gave.
    pub async fn withdraw(
        &self,
        org: OrgId,
        item: Uuid,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let affected = sqlx::query!(
            "UPDATE election_item SET state = 'withdrawn', resolved_at = $3 \
             WHERE org_id = $1 AND id = $2 AND state = 'open'",
            uuid_to_db(org.0),
            uuid_to_db(item),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        if affected == 0 {
            return Err(StorageError::Inconsistent {
                reason: "the item is not open".to_owned(),
            });
        }
        Ok(())
    }

    /// Every standing rule for one tenant, in the form `satisfied_by` reads.
    pub async fn rules(&self, org: OrgId) -> Result<Vec<ElectionRule>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT inventory, axis, trigger_kind, trigger_key, answer_kind, answer, \
                    decided_by, decided_source, decided_user, decided_org, decided_at \
             FROM election_rule WHERE org_id = $1 \
             ORDER BY inventory, axis, trigger_kind, trigger_key",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                let inventory = inventory_from_db(&row.inventory)?;
                let axis = term_kind_from_db(&row.axis)?;
                Ok(ElectionRule {
                    org,
                    inventory,
                    axis,
                    trigger_kind: trigger_kind_from_db(&row.trigger_kind)?,
                    trigger_key: (!row.trigger_key.is_empty()).then_some(row.trigger_key),
                    answer: decode_answer(
                        &row.answer_kind,
                        &row.answer,
                        VocabularyId(inventory, axis),
                    )?,
                    decided_by: decider_from_db(
                        &row.decided_by,
                        row.decided_source,
                        row.decided_user,
                        row.decided_org,
                    )?,
                    decided_at: timestamp_from_db(row.decided_at),
                })
            })
            .collect()
    }

    /// Records a standing answer, replacing whatever the seller said before
    /// about the same question.
    ///
    /// The rule arrives already constructed, because `ElectionRule::new` is
    /// the domain half of the legal backstop and this method must not be a
    /// second way in past it.
    pub async fn upsert_rule(&self, rule: &ElectionRule) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, rule.org).await?;
        write_rule(&mut tx, rule).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// The rule upsert itself, taking the transaction rather than opening one, so
/// an answer that promotes writes both halves atomically and `upsert_rule`
/// stays the one statement.
async fn write_rule(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rule: &ElectionRule,
) -> Result<(), StorageError> {
    let (kind, paths) = encode_answer(&rule.answer);
    let (decided_by, source, user, org_col) = decider_to_db(&rule.decided_by);
    sqlx::query!(
        "INSERT INTO election_rule \
         (org_id, inventory, axis, trigger_kind, trigger_key, answer_kind, answer, \
          decided_by, decided_source, decided_user, decided_org, decided_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
         ON CONFLICT (org_id, inventory, axis, trigger_kind, trigger_key) \
         DO UPDATE SET answer_kind = EXCLUDED.answer_kind, answer = EXCLUDED.answer, \
                       decided_by = EXCLUDED.decided_by, \
                       decided_source = EXCLUDED.decided_source, \
                       decided_user = EXCLUDED.decided_user, \
                       decided_org = EXCLUDED.decided_org, \
                       decided_at = EXCLUDED.decided_at",
        uuid_to_db(rule.org.0),
        inventory_to_db(rule.inventory),
        term_kind_to_db(rule.axis),
        rule.trigger_kind.as_str(),
        rule.trigger_key.clone().unwrap_or_default(),
        kind,
        paths,
        decided_by,
        source,
        user,
        org_col,
        timestamp_to_db(rule.decided_at)?,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The domain constructor, re-exported at the storage boundary so a caller
/// that reaches the repo cannot skip the legal refusal on the way.
pub fn rule_from(request: NewElectionRule) -> Result<ElectionRule, StorageError> {
    ElectionRule::new(request).map_err(|error| StorageError::Inconsistent {
        reason: format!("{error:?}"),
    })
}

fn trigger_kind_from_db(raw: &str) -> Result<ElectionTriggerKind, StorageError> {
    match raw {
        "supply" => Ok(ElectionTriggerKind::Supply),
        "elect_one" => Ok(ElectionTriggerKind::ElectOne),
        "over_cap" => Ok(ElectionTriggerKind::OverCap),
        "narrow" => Ok(ElectionTriggerKind::Narrow),
        other => Err(StorageError::Inconsistent {
            reason: format!("unknown election trigger {other}"),
        }),
    }
}

/// The answered paths as the row stores them. Built by hand rather than
/// derived: `tam-storage` does not depend on `serde` and Phase 4 does not add
/// a dependency to make one encoding shorter.
///
/// The vocabulary is the row's own `(inventory, axis)` and is not repeated.
fn encode_paths(paths: &[VocabularyPath]) -> serde_json::Value {
    serde_json::Value::Array(
        paths
            .iter()
            .map(|path| {
                serde_json::json!({
                    "segments": path.segments,
                    "native_id": path.native_id,
                })
            })
            .collect(),
    )
}

fn decode_paths(
    value: &serde_json::Value,
    vocabulary: VocabularyId,
) -> Result<Vec<VocabularyPath>, StorageError> {
    let malformed = || StorageError::Inconsistent {
        reason: "a stored election answer is an array of {segments, native_id}".to_owned(),
    };
    value
        .as_array()
        .ok_or_else(malformed)?
        .iter()
        .map(|entry| {
            let segments = entry
                .get("segments")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(malformed)?
                .iter()
                .map(|segment| segment.as_str().map(str::to_owned).ok_or_else(malformed))
                .collect::<Result<Vec<String>, StorageError>>()?;
            Ok(VocabularyPath {
                vocabulary,
                segments,
                native_id: entry
                    .get("native_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn encode_answer(answer: &ElectionAnswer) -> (&'static str, serde_json::Value) {
    match answer {
        ElectionAnswer::Value { path } => ("value", encode_paths(core::slice::from_ref(path))),
        ElectionAnswer::Ordering { prefer } => ("ordering", encode_paths(prefer)),
        ElectionAnswer::Delegate => ("delegate", serde_json::Value::Array(Vec::new())),
    }
}

fn decode_answer(
    kind: &str,
    value: &serde_json::Value,
    vocabulary: VocabularyId,
) -> Result<ElectionAnswer, StorageError> {
    match kind {
        "value" => {
            let mut paths = decode_paths(value, vocabulary)?;
            let path = paths.pop().ok_or(StorageError::Inconsistent {
                reason: "a value answer names one path".to_owned(),
            })?;
            Ok(ElectionAnswer::Value { path })
        }
        "ordering" => Ok(ElectionAnswer::Ordering {
            prefer: decode_paths(value, vocabulary)?,
        }),
        "delegate" => Ok(ElectionAnswer::Delegate),
        other => Err(StorageError::Inconsistent {
            reason: format!("unknown election answer {other}"),
        }),
    }
}

/// The answered value of one settled item, decoded against the item's own
/// `(inventory, axis)`.
pub fn answered_paths(
    value: &serde_json::Value,
    vocabulary: VocabularyId,
) -> Result<Vec<VocabularyPath>, StorageError> {
    decode_paths(value, vocabulary)
}
