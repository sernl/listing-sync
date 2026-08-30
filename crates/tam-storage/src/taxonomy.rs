//! The durable half of the taxonomy hub: the global edge relation the pure
//! projection reads, the no-counterpart records that turn `Absent` into an
//! omission, and the per-tenant reconciliation queue. The queue is a drain
//! rather than a treadmill because resolution writes a durable edge or a
//! no-counterpart record, so the second product carrying the same term finds
//! the answer waiting; the partial unique index makes a batch raise one item
//! per gap rather than one per product.

use sqlx::PgPool;
use tam_domain::{
    CanonicalTerm, Decider, EdgeKind, NoCounterpart, ProjectionEdge, TermKind, VocabularyId,
    VocabularyPath,
};
use tam_types::{CanonicalTermId, InventoryId, MappingId, OrgId, Timestamp, Uuid};

use crate::codec::{
    decider_from_db, decider_to_db, edge_kind_from_db, edge_kind_to_db, inventory_from_db,
    inventory_to_db, term_kind_from_db, term_kind_to_db, timestamp_from_db, timestamp_to_db,
    uuid_from_db, uuid_to_db,
};
use crate::jobs::{map_unique, revive_by_gap};
use crate::{pin_org, StorageError};

pub struct TaxonomyRepo {
    pool: PgPool,
}

/// What a seed run did: inserted rows versus rows an earlier run already
/// wrote. Deterministic canonical ids make the split meaningful.
///
/// `ambiguous_terms` is a defect figure rather than an outcome. The
/// single-valued index makes two edges of one kind from one term into one
/// vocabulary impossible, but one `Exact` and one `Broader` onto different
/// paths still projects as `Ambiguous`, and that combination is reachable
/// through the resolution API. Counting it here makes a treadmill a number the
/// drain kill gate reads rather than a silent re-raise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedReport {
    pub terms_inserted: u64,
    pub terms_existing: u64,
    pub edges_inserted: u64,
    pub edges_existing: u64,
    pub ambiguous_terms: u64,
}

/// What a no-counterpart seed run did. Separate from `SeedReport` because
/// three of its four counts would be meaningless here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoCounterpartReport {
    pub inserted: u64,
    pub existing: u64,
}

/// What a raise did per cause: a new item, or a dedup hit on one already open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaiseReport {
    pub new: u64,
    pub already_open: u64,
}

/// The mapping, target inventory and instant a batch of causes is raised
/// under; the causes themselves vary per term and travel separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaiseScope {
    pub mapping: MappingId,
    pub target: InventoryId,
    pub at: Timestamp,
}

/// One open queue item as stored; the resolution calls address it by `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenItem {
    pub id: Uuid,
    pub term: CanonicalTermId,
    pub target: VocabularyId,
    pub raised_by: MappingId,
    pub raised_at: Timestamp,
}

/// The queue counts the drain kill-gate reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DrainStats {
    pub open: u64,
    pub resolved: u64,
    pub no_counterpart: u64,
}

impl TaxonomyRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Idempotent batch upsert of canonical terms and edges. Terms insert
    /// parents before children regardless of input order, because a topic
    /// row references its subject row.
    ///
    /// An upsert rather than an insert-or-ignore: a re-polled vocabulary
    /// renames a target, and under `DO NOTHING` the renamed edge is a second
    /// row beside the first rather than a replacement, which is the one input
    /// the projection reads as `Ambiguous`. The conflict target is the
    /// single-valued index, which excludes `narrower`, so a narrower edge
    /// upserts on its own path instead: a band holds one narrower edge per
    /// year group it covers, and collapsing those onto one key would erase the
    /// candidate list an election is made of.
    pub async fn seed(
        &self,
        terms: &[CanonicalTerm],
        edges: &[ProjectionEdge],
    ) -> Result<SeedReport, StorageError> {
        let mut tx = self.pool.begin().await?;
        let mut report = SeedReport {
            terms_inserted: 0,
            terms_existing: 0,
            edges_inserted: 0,
            edges_existing: 0,
            ambiguous_terms: 0,
        };
        let roots = terms.iter().filter(|term| term.parent.is_none());
        let children = terms.iter().filter(|term| term.parent.is_some());
        for term in roots.chain(children) {
            let row = sqlx::query!(
                "INSERT INTO canonical_term (id, kind, parent, label) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (id) DO UPDATE SET label = EXCLUDED.label, parent = EXCLUDED.parent \
                 RETURNING (xmax = 0) AS \"inserted!\"",
                uuid_to_db(term.id.0),
                term_kind_to_db(term.kind),
                term.parent.map(|parent| uuid_to_db(parent.0)),
                term.label,
            )
            .fetch_one(&mut *tx)
            .await?;
            report.terms_inserted += u64::from(row.inserted);
            report.terms_existing += u64::from(!row.inserted);
        }
        for edge in edges {
            let inserted = upsert_edge(&mut tx, edge).await?;
            report.edges_inserted += u64::from(inserted);
            report.edges_existing += u64::from(!inserted);
        }
        let ambiguous = sqlx::query_scalar!(
            "SELECT count(*) AS \"count!\" FROM ( \
                 SELECT from_term FROM projection_edge WHERE kind <> 'narrower' \
                 GROUP BY from_term, to_inventory, to_term_kind \
                 HAVING count(DISTINCT to_segments) > 1 \
             ) AS ambiguous"
        )
        .fetch_one(&mut *tx)
        .await?;
        report.ambiguous_terms = u64::try_from(ambiguous).unwrap_or(0);
        tx.commit().await?;
        Ok(report)
    }

    /// Records in bulk that terms have no counterpart in a target vocabulary.
    ///
    /// The tenant path (`resolve_no_counterpart`) answers one open queue item;
    /// this is the seeder's path, for an omission a derivation computed rather
    /// than one a seller declared. Without it the four TPT-only grades and the
    /// two year groups no age band covers take the `Absent` branch instead of
    /// the `omitted` branch and become a blocking queue item on every
    /// cross-listing, which is the treadmill the kill gate watches for,
    /// manufactured by the fix.
    pub async fn seed_no_counterparts(
        &self,
        records: &[NoCounterpart],
    ) -> Result<NoCounterpartReport, StorageError> {
        let mut tx = self.pool.begin().await?;
        let mut report = NoCounterpartReport {
            inserted: 0,
            existing: 0,
        };
        for record in records {
            let (decided_by, source, user, org) = decider_to_db(&record.decided_by);
            let VocabularyId(inventory, kind) = record.target;
            let inserted = sqlx::query!(
                "INSERT INTO projection_no_counterpart \
                 (term, to_inventory, to_term_kind, decided_by, decided_source, \
                  decided_user, decided_org, decided_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (term, to_inventory, to_term_kind) DO NOTHING",
                uuid_to_db(record.term.0),
                inventory_to_db(inventory),
                term_kind_to_db(kind),
                decided_by,
                source,
                user,
                org,
                timestamp_to_db(record.decided_at)?,
            )
            .execute(&mut *tx)
            .await?
            .rows_affected();
            report.inserted += inserted;
            report.existing += 1 - inserted;
        }
        tx.commit().await?;
        Ok(report)
    }

    /// Every edge into one target vocabulary, the relation `project` decides
    /// over. Ordered deterministically.
    pub async fn edges_into(
        &self,
        vocabulary: VocabularyId,
    ) -> Result<Vec<ProjectionEdge>, StorageError> {
        let VocabularyId(inventory, kind) = vocabulary;
        let rows = sqlx::query!(
            "SELECT from_term, to_segments, to_native_id, kind, \
                    decided_by, decided_source, decided_user, decided_org, decided_at \
             FROM projection_edge \
             WHERE to_inventory = $1 AND to_term_kind = $2 \
             ORDER BY from_term, kind, to_segments",
            inventory_to_db(inventory),
            term_kind_to_db(kind),
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ProjectionEdge {
                    from: CanonicalTermId(uuid_from_db(row.from_term)),
                    to: VocabularyPath {
                        vocabulary,
                        segments: row.to_segments,
                        native_id: row.to_native_id,
                    },
                    kind: edge_kind_from_db(&row.kind)?,
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

    /// Every edge into several vocabularies at once, which is the shape a
    /// projection actually needs: the caller asks the registry which axes the
    /// target binds and which vocabularies the product's own declarations
    /// name, rather than restating a hardcoded kind list beside every call.
    pub async fn edges_into_all(
        &self,
        vocabularies: &[VocabularyId],
    ) -> Result<Vec<ProjectionEdge>, StorageError> {
        let mut edges = Vec::new();
        for &vocabulary in vocabularies {
            edges.extend(self.edges_into(vocabulary).await?);
        }
        Ok(edges)
    }

    /// The no-counterpart records for one inventory, as the `(term,
    /// vocabulary)` pairs `project_terms` consumes.
    pub async fn no_counterparts_into(
        &self,
        inventory: InventoryId,
    ) -> Result<Vec<(CanonicalTermId, VocabularyId)>, StorageError> {
        let rows = sqlx::query!(
            "SELECT term, to_term_kind FROM projection_no_counterpart \
             WHERE to_inventory = $1 ORDER BY term, to_term_kind",
            inventory_to_db(inventory),
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    CanonicalTermId(uuid_from_db(row.term)),
                    VocabularyId(inventory, term_kind_from_db(&row.to_term_kind)?),
                ))
            })
            .collect()
    }

    /// Raises one item per blocked cause, deduplicated against the open queue
    /// by the partial unique index, so a five-hundred-product batch raises
    /// one item per gap.
    pub async fn raise(
        &self,
        org: OrgId,
        scope: RaiseScope,
        causes: &[(CanonicalTermId, TermKind)],
    ) -> Result<RaiseReport, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let mut report = RaiseReport {
            new: 0,
            already_open: 0,
        };
        for (term, kind) in causes {
            let inserted = sqlx::query!(
                "INSERT INTO reconciliation_item \
                 (org_id, id, term, target_inventory, target_term_kind, \
                  raised_by, raised_at, state) \
                 VALUES ($1, gen_random_uuid(), $2, $3, $4, $5, $6, 'open') \
                 ON CONFLICT (org_id, term, target_inventory, target_term_kind) \
                 WHERE state = 'open' DO NOTHING",
                uuid_to_db(org.0),
                uuid_to_db(term.0),
                inventory_to_db(scope.target),
                term_kind_to_db(*kind),
                uuid_to_db(scope.mapping.0),
                timestamp_to_db(scope.at)?,
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

    pub async fn open_items(&self, org: OrgId) -> Result<Vec<OpenItem>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT id, term, target_inventory, target_term_kind, raised_by, raised_at \
             FROM reconciliation_item \
             WHERE org_id = $1 AND state = 'open' ORDER BY raised_at, id",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(OpenItem {
                    id: uuid_from_db(row.id),
                    term: CanonicalTermId(uuid_from_db(row.term)),
                    target: VocabularyId(
                        inventory_from_db(&row.target_inventory)?,
                        term_kind_from_db(&row.target_term_kind)?,
                    ),
                    raised_by: MappingId(uuid_from_db(row.raised_by)),
                    raised_at: timestamp_from_db(row.raised_at),
                })
            })
            .collect()
    }

    /// Resolution by authoring a durable edge. The edge must answer the item
    /// it resolves — same term, same target vocabulary — and the item must
    /// still be open. A second Exact claim on an already-claimed target path
    /// is rejected by the reverse-uniqueness index, surfaced as
    /// `Inconsistent` rather than resolved at read time.
    ///
    /// The items the answer unblocks are requeued in the same transaction,
    /// keyed on the gate rather than on a mapping: the queue deduplicates per
    /// gap, so one answer releases every item parked behind it, and a
    /// five-hundred-item bulk behind one queue row would otherwise revive one
    /// and leave four hundred and ninety-nine for the park expiry. Returns how
    /// many were released, because a revive that releases nothing is the shape
    /// a silently-unpinned update takes.
    pub async fn resolve_with_edge(
        &self,
        org: OrgId,
        item: Uuid,
        edge: &ProjectionEdge,
    ) -> Result<u64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT term, target_inventory, target_term_kind FROM reconciliation_item \
             WHERE org_id = $1 AND id = $2 AND state = 'open' FOR UPDATE",
            uuid_to_db(org.0),
            uuid_to_db(item),
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StorageError::Inconsistent {
            reason: "the item is not open".to_owned(),
        })?;
        let VocabularyId(edge_inventory, edge_kind) = edge.to.vocabulary;
        if CanonicalTermId(uuid_from_db(row.term)) != edge.from
            || inventory_from_db(&row.target_inventory)? != edge_inventory
            || term_kind_from_db(&row.target_term_kind)? != edge_kind
        {
            return Err(StorageError::Inconsistent {
                reason: "the edge does not answer the item it resolves".to_owned(),
            });
        }
        let (decided_by, source, user, org_col) = decider_to_db(&edge.decided_by);
        map_unique(
            sqlx::query!(
                "INSERT INTO projection_edge \
                 (from_term, to_inventory, to_term_kind, to_segments, to_native_id, \
                  kind, decided_by, decided_source, decided_user, decided_org, decided_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                uuid_to_db(edge.from.0),
                inventory_to_db(edge_inventory),
                term_kind_to_db(edge_kind),
                &edge.to.segments,
                edge.to.native_id.as_deref(),
                edge_kind_to_db(edge.kind),
                decided_by,
                source,
                user,
                org_col,
                timestamp_to_db(edge.decided_at)?,
            )
            .execute(&mut *tx)
            .await,
            "projection_edge_exact_reverse",
            || StorageError::Inconsistent {
                reason: "the target path is already claimed as exact".to_owned(),
            },
        )?;
        sqlx::query!(
            "UPDATE reconciliation_item SET state = 'resolved', resolved_at = $3 \
             WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(item),
            timestamp_to_db(edge.decided_at)?,
        )
        .execute(&mut *tx)
        .await?;
        let revived = revive_by_gap(&mut tx, org, "reconciliation", edge.decided_at).await?;
        tx.commit().await?;
        Ok(revived)
    }

    /// Resolution by recording that the term genuinely has no counterpart:
    /// the durable record is global and idempotent, and the item settles.
    pub async fn resolve_no_counterpart(
        &self,
        org: OrgId,
        item: Uuid,
        decided_by: &Decider,
        at: Timestamp,
    ) -> Result<u64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT term, target_inventory, target_term_kind FROM reconciliation_item \
             WHERE org_id = $1 AND id = $2 AND state = 'open' FOR UPDATE",
            uuid_to_db(org.0),
            uuid_to_db(item),
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StorageError::Inconsistent {
            reason: "the item is not open".to_owned(),
        })?;
        let (decider, source, user, org_col) = decider_to_db(decided_by);
        sqlx::query!(
            "INSERT INTO projection_no_counterpart \
             (term, to_inventory, to_term_kind, decided_by, decided_source, \
              decided_user, decided_org, decided_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (term, to_inventory, to_term_kind) DO NOTHING",
            row.term,
            row.target_inventory,
            row.target_term_kind,
            decider,
            source,
            user,
            org_col,
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE reconciliation_item SET state = 'no_counterpart', resolved_at = $3 \
             WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(item),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        let revived = revive_by_gap(&mut tx, org, "reconciliation", at).await?;
        tx.commit().await?;
        Ok(revived)
    }

    /// The queue counts the drain kill-gate reads per tenant.
    pub async fn drain_stats(&self, org: OrgId) -> Result<DrainStats, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT state, count(*) AS \"count!\" FROM reconciliation_item \
             WHERE org_id = $1 GROUP BY state",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut stats = DrainStats::default();
        for row in rows {
            let count = u64::try_from(row.count).unwrap_or(0);
            match row.state.as_str() {
                "open" => stats.open = count,
                "resolved" => stats.resolved = count,
                "no_counterpart" => stats.no_counterpart = count,
                other => {
                    return Err(StorageError::CorruptRow {
                        reason: format!("unknown reconciliation state {other:?}"),
                    })
                }
            }
        }
        Ok(stats)
    }
}

/// One `canonical_term` row as the relation's own type. Free rather than a
/// method because `query!` mints an anonymous row type per call site, and the
/// two reads below would otherwise decode the same four columns twice.
fn canonical_term(
    id: uuid::Uuid,
    kind: &str,
    parent: Option<uuid::Uuid>,
    label: String,
) -> Result<CanonicalTerm, StorageError> {
    Ok(CanonicalTerm {
        id: CanonicalTermId(uuid_from_db(id)),
        kind: term_kind_from_db(kind)?,
        parent: parent.map(|parent| CanonicalTermId(uuid_from_db(parent))),
        label,
    })
}

impl TaxonomyRepo {
    /// Every canonical term, for the projection's kind lookup. Global
    /// reference data; bounded by the seeded vocabulary's own size.
    pub async fn terms(&self) -> Result<Vec<CanonicalTerm>, StorageError> {
        let rows = sqlx::query!("SELECT id, kind, parent, label FROM canonical_term ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| canonical_term(row.id, &row.kind, row.parent, row.label))
            .collect()
    }

    /// The canonical terms of one axis, or every term where no axis is named,
    /// ordered as a picker reads them rather than as the relation stores them.
    ///
    /// The same global reference data `terms` returns and the same bound on
    /// its size; the kind filter is in the statement rather than the caller
    /// because the subject axis is a fortieth of the relation and the topic
    /// axis is most of the rest.
    pub async fn terms_of_kind(
        &self,
        kind: Option<TermKind>,
    ) -> Result<Vec<CanonicalTerm>, StorageError> {
        let kind = kind.map(term_kind_to_db);
        let rows = sqlx::query!(
            "SELECT id, kind, parent, label FROM canonical_term \
             WHERE $1::text IS NULL OR kind = $1 \
             ORDER BY label, id",
            kind,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| canonical_term(row.id, &row.kind, row.parent, row.label))
            .collect()
    }
}

/// One edge, upserted against whichever key its own kind makes it unique
/// under. An `Exact` or `Broader` edge is single-valued per (term,
/// vocabulary), so its label is refreshed in place; a `Narrower` edge is one
/// of several from that term into that vocabulary, so its own path is the key
/// and the set is added to rather than replaced.
async fn upsert_edge(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    edge: &ProjectionEdge,
) -> Result<bool, StorageError> {
    let (decided_by, source, user, org) = decider_to_db(&edge.decided_by);
    let VocabularyId(inventory, kind) = edge.to.vocabulary;
    let inventory = inventory_to_db(inventory);
    let term_kind = term_kind_to_db(kind);
    let edge_kind = edge_kind_to_db(edge.kind);
    let from = uuid_to_db(edge.from.0);
    let at = timestamp_to_db(edge.decided_at)?;
    let native = edge.to.native_id.as_deref();
    let inserted = match edge.kind {
        EdgeKind::Narrower => {
            sqlx::query!(
                "INSERT INTO projection_edge \
                 (from_term, to_inventory, to_term_kind, to_segments, to_native_id, \
                  kind, decided_by, decided_source, decided_user, decided_org, decided_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
                 ON CONFLICT (from_term, to_inventory, to_term_kind, to_segments, kind) \
                 DO UPDATE SET to_native_id = EXCLUDED.to_native_id \
                 RETURNING (xmax = 0) AS \"inserted!\"",
                from,
                inventory,
                term_kind,
                &edge.to.segments,
                native,
                edge_kind,
                decided_by,
                source,
                user,
                org,
                at,
            )
            .fetch_one(&mut **tx)
            .await?
            .inserted
        }
        EdgeKind::Exact | EdgeKind::Broader => {
            sqlx::query!(
                "INSERT INTO projection_edge \
                 (from_term, to_inventory, to_term_kind, to_segments, to_native_id, \
                  kind, decided_by, decided_source, decided_user, decided_org, decided_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
                 ON CONFLICT (from_term, to_inventory, to_term_kind, kind) \
                 WHERE kind <> 'narrower' \
                 DO UPDATE SET to_segments = EXCLUDED.to_segments, \
                               to_native_id = EXCLUDED.to_native_id \
                 RETURNING (xmax = 0) AS \"inserted!\"",
                from,
                inventory,
                term_kind,
                &edge.to.segments,
                native,
                edge_kind,
                decided_by,
                source,
                user,
                org,
                at,
            )
            .fetch_one(&mut **tx)
            .await?
            .inserted
        }
    };
    Ok(inserted)
}
