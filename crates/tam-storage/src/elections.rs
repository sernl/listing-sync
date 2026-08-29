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
    Election, ElectionAnswer, ElectionRule, ElectionTriggerKind, NewElectionRule,
};
use tam_domain::{TermKind, VocabularyId, VocabularyPath};
use tam_types::{InventoryId, MappingId, OrgId, ProductId, Timestamp, Uuid};

use crate::codec::{
    decider_from_db, decider_to_db, inventory_from_db, inventory_to_db, term_kind_from_db,
    term_kind_to_db, timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db,
};
use crate::jobs::revive_on;
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
}

impl ElectionRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Raises one item per election, deduplicated against the open queue by
    /// the partial unique index.
    ///
    /// The rule check happens before this and is pure, so the common path
    /// writes nothing at all; what reaches here is what no standing answer
    /// covers.
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
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'open') \
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
        item: Uuid,
        paths: &[VocabularyPath],
        at: Timestamp,
    ) -> Result<AnswerReport, StorageError> {
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
        let revived = match row.raised_by {
            Some(mapping) => {
                revive_on(
                    &mut tx,
                    org,
                    MappingId(uuid_from_db(mapping)),
                    "election",
                    at,
                )
                .await?
            }
            None => 0,
        };
        tx.commit().await?;
        Ok(AnswerReport { revived })
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
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
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

/// The answered value of one settled item, for the decision surface's record
/// of what the seller chose.
pub fn answered_paths(
    value: &serde_json::Value,
    vocabulary: VocabularyId,
) -> Result<Vec<VocabularyPath>, StorageError> {
    decode_paths(value, vocabulary)
}
