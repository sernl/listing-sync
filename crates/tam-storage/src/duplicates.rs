//! What was asked about one pair of resources, and what was answered.
//!
//! The pair is unordered and made an order by construction: the lesser
//! identifier is always `product_lo`, so one pair is one row and "we asked
//! already" is a primary-key lookup. That is the whole reason a `different`
//! answer is stored — without it the next import re-asks a question the seller
//! answered, which is the one failure mode a duplicate review cannot survive.
//!
//! The evidence rows follow `field_mismatch`: positioned, classified, one
//! layer per row, with the measured quantity and its unit named beside it
//! rather than six nullable columns.

use sqlx::PgPool;
use tam_types::{OrgId, ProductId, Timestamp, Uuid};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// What was decided about a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Same,
    Different,
    Parked,
}

impl Verdict {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Different => "different",
            Self::Parked => "parked",
        }
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "same" => Ok(Self::Same),
            "different" => Ok(Self::Different),
            "parked" => Ok(Self::Parked),
            other => Err(StorageError::CorruptRow {
                reason: format!("unknown duplicate verdict {other:?}"),
            }),
        }
    }
}

/// Whose claim the verdict is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecidedBy {
    Seller,
    System,
}

impl DecidedBy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Seller => "seller",
            Self::System => "system",
        }
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "seller" => Ok(Self::Seller),
            "system" => Ok(Self::System),
            other => Err(StorageError::CorruptRow {
                reason: format!("unknown duplicate decider {other:?}"),
            }),
        }
    }
}

/// Which layer of the matcher a signal came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchLayer {
    /// An exact payload digest.
    L1,
    /// The overlap of the payload digest sets.
    L1b,
    /// The extracted text's MinHash sketch.
    L2,
    /// The cover's perceptual hash.
    L3,
    /// The normalised title.
    L4,
    /// Metadata corroboration: grades, subject, price, page count.
    L5,
}

impl MatchLayer {
    pub const ALL: [Self; 6] = [Self::L1, Self::L1b, Self::L2, Self::L3, Self::L4, Self::L5];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::L1 => "l1",
            Self::L1b => "l1b",
            Self::L2 => "l2",
            Self::L3 => "l3",
            Self::L4 => "l4",
            Self::L5 => "l5",
        }
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "l1" => Ok(Self::L1),
            "l1b" => Ok(Self::L1b),
            "l2" => Ok(Self::L2),
            "l3" => Ok(Self::L3),
            "l4" => Ok(Self::L4),
            "l5" => Ok(Self::L5),
            other => Err(StorageError::CorruptRow {
                reason: format!("unknown match layer {other:?}"),
            }),
        }
    }
}

/// Whether a signal argued for the pair or against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    Positive,
    Negative,
}

impl Polarity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
        }
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "positive" => Ok(Self::Positive),
            "negative" => Ok(Self::Negative),
            other => Err(StorageError::CorruptRow {
                reason: format!("unknown evidence polarity {other:?}"),
            }),
        }
    }
}

/// What a measure is measured in, named because one column holds them all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceUnit {
    Jaccard,
    Hamming,
    Bytes,
    Ratio,
    Count,
}

impl EvidenceUnit {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Jaccard => "jaccard",
            Self::Hamming => "hamming",
            Self::Bytes => "bytes",
            Self::Ratio => "ratio",
            Self::Count => "count",
        }
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "jaccard" => Ok(Self::Jaccard),
            "hamming" => Ok(Self::Hamming),
            "bytes" => Ok(Self::Bytes),
            "ratio" => Ok(Self::Ratio),
            "count" => Ok(Self::Count),
            other => Err(StorageError::CorruptRow {
                reason: format!("unknown evidence unit {other:?}"),
            }),
        }
    }
}

/// One layer's finding about one pair.
#[derive(Debug, Clone, PartialEq)]
pub struct Evidence {
    pub layer: MatchLayer,
    pub polarity: Polarity,
    pub measure: f32,
    pub unit: EvidenceUnit,
    /// What the value was, where naming it is what makes the seller's sentence
    /// concrete.
    pub observed_in: Option<String>,
}

/// A pair to record.
#[derive(Debug, Clone)]
pub struct NewVerdict<'a> {
    pub lo: ProductId,
    pub hi: ProductId,
    pub verdict: Verdict,
    pub decided_by: DecidedBy,
    pub winning_layer: MatchLayer,
    pub log_odds: f32,
    pub fingerprint_version: i16,
    pub run: Option<Uuid>,
    pub kept: Option<ProductId>,
    pub raised_at: Timestamp,
    pub decided_at: Option<Timestamp>,
    pub reversible_until: Option<Timestamp>,
    pub evidence: &'a [Evidence],
}

/// One pair as it stands.
#[derive(Debug, Clone)]
pub struct VerdictRecord {
    pub lo: ProductId,
    pub hi: ProductId,
    pub verdict: Verdict,
    pub decided_by: DecidedBy,
    pub winning_layer: MatchLayer,
    pub log_odds: f32,
    pub fingerprint_version: i16,
    pub run: Option<Uuid>,
    pub kept: Option<ProductId>,
    pub raised_at: Timestamp,
    pub decided_at: Option<Timestamp>,
    pub reversible_until: Option<Timestamp>,
    pub evidence: Vec<Evidence>,
}

/// How long a merge stays reversible, as milliseconds.
///
/// Thirty days, which is the figure the seller is told and Amazon's own window
/// for the same question. Stored per row rather than computed on read, for
/// `import_batch.expires_at`'s reason: it is the deadline they were given.
pub const REVERSIBLE_MS: i64 = 30 * 24 * 60 * 60 * 1000;

pub struct DuplicateRepo {
    pool: PgPool,
}

impl DuplicateRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Records a pair the matcher raised, leaving a pair already answered
    /// alone.
    ///
    /// The conflict clause is the whole of the never-ask-twice rule: a seller
    /// who said `different` last month has their answer standing, and the
    /// import that would have asked again reads it instead. Answers whether
    /// this write is what created the row.
    pub async fn raise(&self, org: OrgId, new: &NewVerdict<'_>) -> Result<bool, StorageError> {
        let (lo, hi) = ordered(new.lo, new.hi);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let inserted = sqlx::query!(
            "INSERT INTO duplicate_verdict \
               (org_id, product_lo, product_hi, verdict, decided_by, winning_layer, \
                log_odds, fingerprint_version, run_id, kept_product, raised_at, \
                decided_at, reversible_until) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
             ON CONFLICT (org_id, product_lo, product_hi) DO NOTHING",
            uuid_to_db(org.0),
            uuid_to_db(lo.0),
            uuid_to_db(hi.0),
            new.verdict.as_str(),
            new.decided_by.as_str(),
            new.winning_layer.as_str(),
            new.log_odds,
            new.fingerprint_version,
            new.run.map(uuid_to_db),
            new.kept.map(|kept| uuid_to_db(kept.0)),
            timestamp_to_db(new.raised_at)?,
            new.decided_at.map(timestamp_to_db).transpose()?,
            new.reversible_until.map(timestamp_to_db).transpose()?,
        )
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            tx.commit().await?;
            return Ok(false);
        }
        for (position, evidence) in new.evidence.iter().enumerate() {
            sqlx::query!(
                "INSERT INTO duplicate_evidence \
                   (org_id, product_lo, product_hi, position, layer, polarity, measure, \
                    unit, observed_in) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 ON CONFLICT (org_id, product_lo, product_hi, layer) DO NOTHING",
                uuid_to_db(org.0),
                uuid_to_db(lo.0),
                uuid_to_db(hi.0),
                i32::try_from(position).unwrap_or(i32::MAX),
                evidence.layer.as_str(),
                evidence.polarity.as_str(),
                evidence.measure,
                evidence.unit.as_str(),
                evidence.observed_in.as_deref(),
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    /// The seller's answer, or the reversal of one.
    #[expect(
        clippy::too_many_arguments,
        reason = "the pair is two identifiers by construction -- the column CHECK states the \
                  order -- and folding them into a struct would name a parameter bag whose \
                  whole content is already this table's primary key"
    )]
    pub async fn decide(
        &self,
        org: OrgId,
        lo: ProductId,
        hi: ProductId,
        verdict: Verdict,
        kept: Option<ProductId>,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let (lo, hi) = ordered(lo, hi);
        let decided = match verdict {
            Verdict::Parked => None,
            Verdict::Same | Verdict::Different => Some(timestamp_to_db(at)?),
        };
        let reversible = match verdict {
            Verdict::Same => Some(timestamp_to_db(Timestamp(
                at.0.saturating_add(REVERSIBLE_MS),
            ))?),
            Verdict::Different | Verdict::Parked => None,
        };
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let moved = sqlx::query!(
            "UPDATE duplicate_verdict \
                SET verdict = $4, decided_by = 'seller', kept_product = $5, \
                    decided_at = $6, reversible_until = $7 \
              WHERE org_id = $1 AND product_lo = $2 AND product_hi = $3",
            uuid_to_db(org.0),
            uuid_to_db(lo.0),
            uuid_to_db(hi.0),
            verdict.as_str(),
            kept.map(|kept| uuid_to_db(kept.0)),
            decided,
            reversible,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(moved.rows_affected() > 0)
    }

    /// One pair, with its evidence.
    pub async fn get(
        &self,
        org: OrgId,
        lo: ProductId,
        hi: ProductId,
    ) -> Result<Option<VerdictRecord>, StorageError> {
        let (lo, hi) = ordered(lo, hi);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT verdict, decided_by, winning_layer, log_odds, fingerprint_version, \
                    run_id, kept_product, raised_at, decided_at, reversible_until \
               FROM duplicate_verdict \
              WHERE org_id = $1 AND product_lo = $2 AND product_hi = $3",
            uuid_to_db(org.0),
            uuid_to_db(lo.0),
            uuid_to_db(hi.0),
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        let evidence = sqlx::query!(
            "SELECT layer, polarity, measure, unit, observed_in FROM duplicate_evidence \
              WHERE org_id = $1 AND product_lo = $2 AND product_hi = $3 ORDER BY position",
            uuid_to_db(org.0),
            uuid_to_db(lo.0),
            uuid_to_db(hi.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(VerdictRecord {
            lo,
            hi,
            verdict: Verdict::from_db(&row.verdict)?,
            decided_by: DecidedBy::from_db(&row.decided_by)?,
            winning_layer: MatchLayer::from_db(&row.winning_layer)?,
            log_odds: row.log_odds,
            fingerprint_version: row.fingerprint_version,
            run: row.run_id.map(uuid_from_db),
            kept: row.kept_product.map(|kept| ProductId(uuid_from_db(kept))),
            raised_at: timestamp_from_db(row.raised_at),
            decided_at: row.decided_at.map(timestamp_from_db),
            reversible_until: row.reversible_until.map(timestamp_from_db),
            evidence: evidence
                .into_iter()
                .map(|row| {
                    Ok(Evidence {
                        layer: MatchLayer::from_db(&row.layer)?,
                        polarity: Polarity::from_db(&row.polarity)?,
                        measure: row.measure,
                        unit: EvidenceUnit::from_db(&row.unit)?,
                        observed_in: row.observed_in,
                    })
                })
                .collect::<Result<Vec<_>, StorageError>>()?,
        }))
    }

    /// The pairs still waiting on the seller, for one run or for the whole
    /// organisation.
    pub async fn open_pairs(
        &self,
        org: OrgId,
        run: Option<Uuid>,
    ) -> Result<Vec<VerdictRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT product_lo, product_hi, verdict, decided_by, winning_layer, log_odds, \
                    fingerprint_version, run_id, kept_product, raised_at, decided_at, \
                    reversible_until \
               FROM duplicate_verdict \
              WHERE org_id = $1 AND verdict = 'parked' \
                AND ($2::uuid IS NULL OR run_id = $2) \
              ORDER BY raised_at, product_lo, product_hi",
            uuid_to_db(org.0),
            run.map(uuid_to_db),
        )
        .fetch_all(&mut *tx)
        .await?;
        let evidence = sqlx::query!(
            "SELECT e.product_lo, e.product_hi, e.layer, e.polarity, e.measure, e.unit, \
                    e.observed_in \
               FROM duplicate_evidence e \
               JOIN duplicate_verdict v ON v.org_id = e.org_id \
                AND v.product_lo = e.product_lo AND v.product_hi = e.product_hi \
              WHERE e.org_id = $1 AND v.verdict = 'parked' \
                AND ($2::uuid IS NULL OR v.run_id = $2) \
              ORDER BY e.position",
            uuid_to_db(org.0),
            run.map(uuid_to_db),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let lo = ProductId(uuid_from_db(row.product_lo));
            let hi = ProductId(uuid_from_db(row.product_hi));
            let mut held = Vec::new();
            for found in evidence
                .iter()
                .filter(|e| e.product_lo == row.product_lo && e.product_hi == row.product_hi)
            {
                held.push(Evidence {
                    layer: MatchLayer::from_db(&found.layer)?,
                    polarity: Polarity::from_db(&found.polarity)?,
                    measure: found.measure,
                    unit: EvidenceUnit::from_db(&found.unit)?,
                    observed_in: found.observed_in.clone(),
                });
            }
            out.push(VerdictRecord {
                lo,
                hi,
                verdict: Verdict::from_db(&row.verdict)?,
                decided_by: DecidedBy::from_db(&row.decided_by)?,
                winning_layer: MatchLayer::from_db(&row.winning_layer)?,
                log_odds: row.log_odds,
                fingerprint_version: row.fingerprint_version,
                run: row.run_id.map(uuid_from_db),
                kept: row.kept_product.map(|kept| ProductId(uuid_from_db(kept))),
                raised_at: timestamp_from_db(row.raised_at),
                decided_at: row.decided_at.map(timestamp_from_db),
                reversible_until: row.reversible_until.map(timestamp_from_db),
                evidence: held,
            });
        }
        Ok(out)
    }

    /// How many questions are still open about one resource.
    ///
    /// The predicate an import run item is unblocked by: an item is held in
    /// review by the pairs it is in, so answering the last of them is what
    /// lets the commit reach it. Counted rather than read as a list, because
    /// the caller's only question is whether any remain.
    pub async fn parked_for(&self, org: OrgId, product: ProductId) -> Result<u32, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let held: i64 = sqlx::query_scalar!(
            "SELECT count(*)::bigint FROM duplicate_verdict \
              WHERE org_id = $1 AND verdict = 'parked' \
                AND (product_lo = $2 OR product_hi = $2)",
            uuid_to_db(org.0),
            uuid_to_db(product.0),
        )
        .fetch_one(&mut *tx)
        .await?
        .unwrap_or(0);
        tx.commit().await?;
        Ok(u32::try_from(held).unwrap_or(u32::MAX))
    }

    /// Which of these pairs have an answer already, so the matcher does not
    /// raise a question twice.
    ///
    /// Every verdict, not only the settled ones: a pair the seller has parked
    /// is a question already on their desk, and raising it again would put two
    /// cards up for one pair.
    pub async fn answered(
        &self,
        org: OrgId,
        pairs: &[(ProductId, ProductId)],
    ) -> Result<Vec<(ProductId, ProductId, Verdict)>, StorageError> {
        if pairs.is_empty() {
            return Ok(Vec::new());
        }
        let mut los = Vec::with_capacity(pairs.len());
        let mut his = Vec::with_capacity(pairs.len());
        for (lo, hi) in pairs {
            let (lo, hi) = ordered(*lo, *hi);
            los.push(uuid_to_db(lo.0));
            his.push(uuid_to_db(hi.0));
        }
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT product_lo, product_hi, verdict FROM duplicate_verdict \
              WHERE org_id = $1 AND (product_lo, product_hi) \
                    IN (SELECT * FROM unnest($2::uuid[], $3::uuid[]))",
            uuid_to_db(org.0),
            &los,
            &his,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    ProductId(uuid_from_db(row.product_lo)),
                    ProductId(uuid_from_db(row.product_hi)),
                    Verdict::from_db(&row.verdict)?,
                ))
            })
            .collect()
    }
}

/// The pair's canonical order, which the column CHECK also states: one pair is
/// one row, so the caller never has to remember which way round it asked.
#[must_use]
pub fn ordered(a: ProductId, b: ProductId) -> (ProductId, ProductId) {
    if a.0 .0 <= b.0 .0 {
        (a, b)
    } else {
        (b, a)
    }
}
