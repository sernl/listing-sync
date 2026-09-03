//! The durable half of the per-seller override layer.
//!
//! `projection_edge` is global reference data because an edge is a fact about
//! two vocabularies. An override is a decision about one catalogue, so it is
//! tenant data under the forced null-safe policy, and every statement here
//! pins `app.current_org` before it runs: an unpinned statement under forced
//! row-level security matches no rows and answers as though a fence had
//! refused it, which is indistinguishable from an empty result.
//!
//! Reads return the whole org's set rather than one axis's, because the
//! projection loads a product's context once and consults it per axis, and a
//! seller has tens of overrides rather than thousands.

use sqlx::PgPool;
use tam_domain::equivalence::{OverrideKind, ProjectionOverride};
use tam_domain::{TermKind, VocabularyId, VocabularyPath};
use tam_types::{CanonicalTermId, InventoryId, OrgId};

use crate::codec::{
    decider_from_db, decider_to_db, inventory_from_db, inventory_to_db, term_kind_from_db,
    term_kind_to_db, timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db,
};
use crate::{pin_org, StorageError};

pub struct OverrideRepo {
    pool: PgPool,
}

impl OverrideRepo {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Records one seller's decision, replacing whatever they said before
    /// about the same term and target.
    ///
    /// The override arrives already constructed, because
    /// `ProjectionOverride::new` is the domain half of the licence refusal and
    /// this method must not become a second way past it.
    pub async fn upsert(&self, entry: &ProjectionOverride) -> Result<(), StorageError> {
        let (decided_by, source, user, org_column) = decider_to_db(&entry.decided_by);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, entry.org).await?;
        sqlx::query!(
            "INSERT INTO projection_override \
             (org_id, inventory, axis, from_term, to_segments, to_native_id, kind, \
              decided_by, decided_source, decided_user, decided_org, decided_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
             ON CONFLICT (org_id, inventory, axis, from_term) \
             DO UPDATE SET to_segments = EXCLUDED.to_segments, \
                           to_native_id = EXCLUDED.to_native_id, \
                           kind = EXCLUDED.kind, \
                           decided_by = EXCLUDED.decided_by, \
                           decided_source = EXCLUDED.decided_source, \
                           decided_user = EXCLUDED.decided_user, \
                           decided_org = EXCLUDED.decided_org, \
                           decided_at = EXCLUDED.decided_at",
            uuid_to_db(entry.org.0),
            inventory_to_db(entry.inventory),
            term_kind_to_db(entry.axis),
            uuid_to_db(entry.from.0),
            &entry.to.segments,
            entry.to.native_id.as_deref(),
            override_kind_to_db(entry.kind),
            decided_by,
            source,
            user,
            org_column,
            timestamp_to_db(entry.decided_at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Every override one organisation holds, ordered deterministically so a
    /// projection reads the same relation twice running.
    pub async fn for_org(&self, org: OrgId) -> Result<Vec<ProjectionOverride>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT inventory, axis, from_term, to_segments, to_native_id, kind, \
                    decided_by, decided_source, decided_user, decided_org, decided_at \
             FROM projection_override WHERE org_id = $1 \
             ORDER BY inventory, axis, from_term",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                let inventory = inventory_from_db(&row.inventory)?;
                let axis = term_kind_from_db(&row.axis)?;
                Ok(ProjectionOverride {
                    org,
                    inventory,
                    axis,
                    from: CanonicalTermId(uuid_from_db(row.from_term)),
                    to: VocabularyPath {
                        vocabulary: VocabularyId(inventory, axis),
                        segments: row.to_segments,
                        native_id: row.to_native_id,
                    },
                    kind: override_kind_from_db(&row.kind)?,
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

    /// Withdraws one decision, returning whether a row was there to withdraw.
    ///
    /// The boolean is returned rather than discarded because a delete that
    /// matched nothing and a delete that matched are different facts to a
    /// seller who has just pressed remove, and under forced row-level
    /// security they are also what a wrong tenant scope looks like.
    pub async fn remove(
        &self,
        org: OrgId,
        inventory: InventoryId,
        axis: TermKind,
        from: CanonicalTermId,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let affected = sqlx::query!(
            "DELETE FROM projection_override \
             WHERE org_id = $1 AND inventory = $2 AND axis = $3 AND from_term = $4",
            uuid_to_db(org.0),
            inventory_to_db(inventory),
            term_kind_to_db(axis),
            uuid_to_db(from.0),
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(affected > 0)
    }
}

/// The two projecting kinds, spelled as the table's CHECK spells them.
/// `EdgeKind`'s codec is not reused: it admits `narrower`, which this column
/// refuses, and sharing the codec would make the refusal depend on the caller
/// rather than on the type.
const fn override_kind_to_db(kind: OverrideKind) -> &'static str {
    match kind {
        OverrideKind::Exact => "exact",
        OverrideKind::Broader => "broader",
    }
}

fn override_kind_from_db(raw: &str) -> Result<OverrideKind, StorageError> {
    match raw {
        "exact" => Ok(OverrideKind::Exact),
        "broader" => Ok(OverrideKind::Broader),
        other => Err(StorageError::CorruptRow {
            reason: format!(
                "projection_override.kind holds {other}, which is neither exact nor broader"
            ),
        }),
    }
}
