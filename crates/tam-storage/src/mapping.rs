//! The mapping aggregate: `Binding`, `Verification`, `FieldPolicies`,
//! `PriceRule` and `RemoteLifecycle` rendered to columns and back, totally in
//! both directions. Encoding is by construction (a well-typed `Mapping`
//! always encodes); decoding is defensive, naming the corrupt column set
//! rather than panicking on it.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tam_domain::equivalence::{Loss, LossKind};
use tam_domain::{
    Binding, Decider, FieldPolicies, FieldPolicy, Mapping, PublishMode, SeverCause, TermKind,
    Verification, VocabularyPath,
};
use tam_marketplace::{CorrelationMarker, RemoteLifecycle, RemoteListingId};
use tam_types::{
    AttemptId, FieldKey, FieldMismatch, InventoryId, MappingId, MismatchClass, OrgId, PriceIntent,
    PriceRule, ProductId, Rounding, Timestamp,
};

use crate::codec::{
    inventory_from_db, inventory_to_db, marketplace_to_db, price_from_db, term_kind_from_db,
    term_kind_to_db, timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db, PriceColumns,
    RemoteIdColumns, PRICE_KIND_FREE, PRICE_KIND_PAID,
};
use crate::{pin_org, StorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingRecord {
    pub mapping: Mapping,
    pub normaliser_version: u32,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Two mappings must not claim one listing, and the path that reaches this is
/// an ordinary seller one: a migrate mints a fresh product per read and
/// dedupes by nothing, so the same source listing submitted twice under two
/// idempotency keys raises the second bound claim. Named rather than left as
/// a bare unique violation, which surfaces as a 500 for what is a validation
/// answer.
fn map_bound_claim<T>(outcome: Result<T, sqlx::Error>) -> Result<T, StorageError> {
    match outcome {
        Ok(value) => Ok(value),
        Err(sqlx::Error::Database(database))
            if matches!(
                database.constraint(),
                Some("mapping_one_bound_url" | "mapping_one_bound_numeric_id")
            ) =>
        {
            Err(StorageError::ListingAlreadyBound)
        }
        Err(error) => Err(error.into()),
    }
}

pub struct MappingRepo {
    pool: PgPool,
}

impl MappingRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(
        &self,
        org: OrgId,
        mapping: &Mapping,
        normaliser_version: u32,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        if mapping.org != org {
            return Err(StorageError::OrgMismatch);
        }
        let org_db = uuid_to_db(org.0);
        let mapping_db = uuid_to_db(mapping.id.0);
        let at_db = timestamp_to_db(at)?;
        let binding = BindingColumns::encode(&mapping.binding)?;
        let verify = VerifyColumns::encode(&mapping.binding)?;
        let price = PriceRuleColumns::encode(mapping.price_rule);
        let lifecycle = LifecycleColumns::encode(&mapping.lifecycle)?;
        let normaliser =
            i32::try_from(normaliser_version).map_err(|_| StorageError::Inconsistent {
                reason: format!("normaliser version {normaliser_version} exceeds the column range"),
            })?;

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;

        let inserted = sqlx::query!(
            "INSERT INTO mapping \
             (org_id, id, product_id, inventory, marketplace, \
              binding_state, remote_id_kind, remote_url, remote_numeric_id, \
              binding_attempt, binding_marker, first_seen_at, ambiguous_since, \
              severed_at, sever_cause, \
              verify_state, verified_at, verify_stale_since, normaliser_version, \
              policy_title, policy_description, policy_price, policy_taxonomy, \
              policy_grades, policy_files, \
              price_rule_kind, price_rate_micros, price_rounding, \
              price_explicit_kind, price_explicit_minor_units, price_explicit_currency, \
              publish_mode, lifecycle_state, lifecycle_since, lifecycle_reason, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, \
                     $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, \
                     $29, $30, $31, $32, $33, $34, $35, $36, $37)",
            org_db,
            mapping_db,
            uuid_to_db(mapping.product.0),
            inventory_to_db(mapping.inventory),
            marketplace_to_db(mapping.inventory.marketplace()),
            binding.state,
            binding.remote_id_kind,
            binding.remote_url,
            binding.remote_numeric_id,
            binding.attempt,
            binding.marker,
            binding.first_seen_at,
            binding.ambiguous_since,
            binding.severed_at,
            binding.sever_cause,
            verify.state,
            verify.verified_at,
            verify.stale_since,
            normaliser,
            policy_to_db(mapping.policies.title),
            policy_to_db(mapping.policies.description),
            policy_to_db(mapping.policies.price),
            policy_to_db(mapping.policies.taxonomy),
            policy_to_db(mapping.policies.grades),
            policy_to_db(mapping.policies.files),
            price.kind,
            price.rate_micros,
            price.rounding,
            price.explicit_kind,
            price.explicit_minor_units,
            price.explicit_currency,
            publish_to_db(mapping.publish),
            lifecycle.state,
            lifecycle.since,
            lifecycle.reason,
            at_db,
            at_db,
        )
        .execute(&mut *tx)
        .await;
        map_bound_claim(inserted)?;

        for (position, mismatch) in verify.mismatches.iter().enumerate() {
            let row = MismatchColumns::encode(mismatch)?;
            let position = i32::try_from(position).map_err(|_| StorageError::Inconsistent {
                reason: format!("mismatch position {position} exceeds the column range"),
            })?;
            sqlx::query!(
                "INSERT INTO field_mismatch \
                 (org_id, mapping_id, position, field, class, observed_in, limit_observed) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                org_db,
                mapping_db,
                position,
                row.field,
                row.class,
                row.observed_in,
                row.limit_observed,
            )
            .execute(&mut *tx)
            .await?;
        }

        for (position, candidate) in binding.candidates.iter().enumerate() {
            let row = RemoteIdColumns::encode(candidate)?;
            let position = i32::try_from(position).map_err(|_| StorageError::Inconsistent {
                reason: format!("candidate position {position} exceeds the column range"),
            })?;
            sqlx::query!(
                "INSERT INTO binding_candidate \
                 (org_id, mapping_id, position, remote_id_kind, remote_url, remote_numeric_id) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
                org_db,
                mapping_db,
                position,
                row.kind,
                row.url,
                row.numeric_id,
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn get(
        &self,
        org: OrgId,
        id: MappingId,
    ) -> Result<Option<MappingRecord>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let mapping_db = uuid_to_db(id.0);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query_as!(
            MappingRow,
            "SELECT org_id, id, product_id, inventory, binding_state, remote_id_kind, \
             remote_url, remote_numeric_id, binding_attempt, binding_marker, first_seen_at, \
             ambiguous_since, severed_at, sever_cause, verify_state, verified_at, \
             verify_stale_since, normaliser_version, policy_title, policy_description, \
             policy_price, policy_taxonomy, policy_grades, policy_files, price_rule_kind, \
             price_rate_micros, price_rounding, price_explicit_kind, \
             price_explicit_minor_units, price_explicit_currency, publish_mode, \
             lifecycle_state, lifecycle_since, lifecycle_reason, created_at, updated_at \
             FROM mapping WHERE org_id = $1 AND id = $2",
            org_db,
            mapping_db,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        let record = hydrate(&mut tx, org_db, row).await?;
        tx.commit().await?;
        Ok(Some(record))
    }

    pub async fn list_for_product(
        &self,
        org: OrgId,
        product: ProductId,
    ) -> Result<Vec<MappingRecord>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let product_db = uuid_to_db(product.0);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query_as!(
            MappingRow,
            "SELECT org_id, id, product_id, inventory, binding_state, remote_id_kind, \
             remote_url, remote_numeric_id, binding_attempt, binding_marker, first_seen_at, \
             ambiguous_since, severed_at, sever_cause, verify_state, verified_at, \
             verify_stale_since, normaliser_version, policy_title, policy_description, \
             policy_price, policy_taxonomy, policy_grades, policy_files, price_rule_kind, \
             price_rate_micros, price_rounding, price_explicit_kind, \
             price_explicit_minor_units, price_explicit_currency, publish_mode, \
             lifecycle_state, lifecycle_since, lifecycle_reason, created_at, updated_at \
             FROM mapping WHERE org_id = $1 AND product_id = $2 \
             ORDER BY inventory",
            org_db,
            product_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            records.push(hydrate(&mut tx, org_db, row).await?);
        }
        tx.commit().await?;
        Ok(records)
    }
}

async fn hydrate(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    row: MappingRow,
) -> Result<MappingRecord, StorageError> {
    let mapping_db = row.id;
    let mismatches = sqlx::query_as!(
        MismatchRow,
        "SELECT field, class, observed_in, limit_observed \
         FROM field_mismatch WHERE org_id = $1 AND mapping_id = $2 \
         ORDER BY position",
        org_db,
        mapping_db,
    )
    .fetch_all(&mut **tx)
    .await?;
    let candidates = sqlx::query_as!(
        CandidateRow,
        "SELECT remote_id_kind AS kind, remote_url AS url, remote_numeric_id AS numeric_id \
         FROM binding_candidate WHERE org_id = $1 AND mapping_id = $2 \
         ORDER BY position",
        org_db,
        mapping_db,
    )
    .fetch_all(&mut **tx)
    .await?;
    decode_mapping(&row, &mismatches, candidates)
}

struct BindingColumns<'a> {
    state: &'static str,
    remote_id_kind: Option<&'static str>,
    remote_url: Option<&'a str>,
    remote_numeric_id: Option<i64>,
    attempt: Option<uuid::Uuid>,
    marker: Option<&'a str>,
    first_seen_at: Option<DateTime<Utc>>,
    ambiguous_since: Option<DateTime<Utc>>,
    severed_at: Option<DateTime<Utc>>,
    sever_cause: Option<&'static str>,
    candidates: &'a [RemoteListingId],
}

impl<'a> BindingColumns<'a> {
    fn encode(binding: &'a Binding) -> Result<Self, StorageError> {
        let empty = Self {
            state: "unbound",
            remote_id_kind: None,
            remote_url: None,
            remote_numeric_id: None,
            attempt: None,
            marker: None,
            first_seen_at: None,
            ambiguous_since: None,
            severed_at: None,
            sever_cause: None,
            candidates: &[],
        };
        Ok(match binding {
            Binding::Unbound => empty,
            Binding::Creating { attempt, marker } => Self {
                state: "creating",
                attempt: Some(uuid_to_db(attempt.0)),
                marker: marker.as_ref().map(|m| m.0.as_str()),
                ..empty
            },
            Binding::Bound { id, first_seen, .. } => {
                let remote = RemoteIdColumns::encode(id)?;
                Self {
                    state: "bound",
                    remote_id_kind: Some(remote.kind),
                    remote_url: remote.url,
                    remote_numeric_id: remote.numeric_id,
                    first_seen_at: Some(timestamp_to_db(*first_seen)?),
                    ..empty
                }
            }
            Binding::AmbiguousCreate {
                attempt,
                candidates,
                since,
            } => Self {
                state: "ambiguous_create",
                attempt: Some(uuid_to_db(attempt.0)),
                ambiguous_since: Some(timestamp_to_db(*since)?),
                candidates: candidates.as_slice(),
                ..empty
            },
            Binding::Severed {
                was,
                noticed,
                cause,
            } => {
                let remote = RemoteIdColumns::encode(was)?;
                Self {
                    state: "severed",
                    remote_id_kind: Some(remote.kind),
                    remote_url: remote.url,
                    remote_numeric_id: remote.numeric_id,
                    severed_at: Some(timestamp_to_db(*noticed)?),
                    sever_cause: Some(sever_to_db(*cause)),
                    ..empty
                }
            }
        })
    }
}

struct VerifyColumns<'a> {
    state: &'static str,
    verified_at: Option<DateTime<Utc>>,
    stale_since: Option<DateTime<Utc>>,
    mismatches: Vec<&'a FieldMismatch>,
}

impl<'a> VerifyColumns<'a> {
    fn encode(binding: &'a Binding) -> Result<Self, StorageError> {
        let neutral = Self {
            state: "stale",
            verified_at: None,
            stale_since: None,
            mismatches: Vec::new(),
        };
        let Binding::Bound { verified, .. } = binding else {
            return Ok(neutral);
        };
        Ok(match verified {
            Verification::Stale { since } => Self {
                stale_since: Some(timestamp_to_db(*since)?),
                ..neutral
            },
            Verification::Clean { at } => Self {
                state: "clean",
                verified_at: Some(timestamp_to_db(*at)?),
                ..neutral
            },
            Verification::Mismatched { at, first, rest } => Self {
                state: "mismatched",
                verified_at: Some(timestamp_to_db(*at)?),
                stale_since: None,
                mismatches: core::iter::once(first).chain(rest.iter()).collect(),
            },
        })
    }
}

pub(crate) fn remote_id_from_db(
    kind: &str,
    url: Option<String>,
    numeric_id: Option<i64>,
) -> Result<RemoteListingId, StorageError> {
    match (kind, url, numeric_id) {
        ("tes", Some(url), None) => Ok(RemoteListingId::Tes { url }),
        ("tpt", None, Some(id)) => Ok(RemoteListingId::Tpt {
            product_id: u64::try_from(id).map_err(|_| StorageError::CorruptRow {
                reason: format!("negative tpt product id {id}"),
            })?,
        }),
        ("etsy", None, Some(id)) => Ok(RemoteListingId::Etsy {
            listing_id: u64::try_from(id).map_err(|_| StorageError::CorruptRow {
                reason: format!("negative etsy listing id {id}"),
            })?,
        }),
        (kind, url, numeric_id) => Err(StorageError::CorruptRow {
            reason: format!("inconsistent remote id columns ({kind:?}, {url:?}, {numeric_id:?})"),
        }),
    }
}

struct PriceRuleColumns {
    kind: &'static str,
    rate_micros: Option<i64>,
    rounding: Option<&'static str>,
    explicit_kind: Option<&'static str>,
    explicit_minor_units: Option<i64>,
    explicit_currency: Option<&'static str>,
}

impl PriceRuleColumns {
    fn encode(rule: PriceRule) -> Self {
        match rule {
            PriceRule::Converted {
                rate_micros,
                rounding,
            } => Self {
                kind: "converted",
                rate_micros: Some(rate_micros),
                rounding: Some(match rounding {
                    Rounding::Nearest => "nearest",
                    Rounding::UpToCharm => "up_to_charm",
                }),
                explicit_kind: None,
                explicit_minor_units: None,
                explicit_currency: None,
            },
            PriceRule::Explicit(intent) => {
                let price = PriceColumns::from_intent(intent);
                Self {
                    kind: "explicit",
                    rate_micros: None,
                    rounding: None,
                    explicit_kind: Some(price.kind),
                    explicit_minor_units: price.minor_units,
                    explicit_currency: price.currency,
                }
            }
        }
    }
}

/// The three columns `mapping_lifecycle_total` constrains together, and the
/// only encoder of them. Every writer of a mapping's lifecycle goes through
/// it, because the constraint's third clause -- `lifecycle_reason IS NULL OR
/// lifecycle_state = 'rejected'` -- is a clause a hand-written statement will
/// forget, and forgetting it aborts the settling transaction and strands the
/// attempt `in_flight`, which `write_attempt_one_in_flight` then turns into a
/// permanent refusal of every future attempt on that mapping.
pub(crate) struct LifecycleColumns {
    pub(crate) state: &'static str,
    pub(crate) since: Option<DateTime<Utc>>,
    pub(crate) reason: Option<String>,
}

impl LifecycleColumns {
    pub(crate) fn encode(lifecycle: &RemoteLifecycle) -> Result<Self, StorageError> {
        let (state, since, reason) = match lifecycle {
            RemoteLifecycle::Absent => ("absent", None, None),
            RemoteLifecycle::Draft => ("draft", None, None),
            RemoteLifecycle::Submitted { at } => ("submitted", Some(*at), None),
            RemoteLifecycle::InReview { since } => ("in_review", Some(*since), None),
            RemoteLifecycle::Live { since } => ("live", Some(*since), None),
            RemoteLifecycle::Rejected { at, reason } => ("rejected", Some(*at), reason.clone()),
            RemoteLifecycle::Withdrawn { at } => ("withdrawn", Some(*at), None),
        };
        Ok(Self {
            state,
            since: since.map(timestamp_to_db).transpose()?,
            reason,
        })
    }
}

fn lifecycle_from_db(
    state: &str,
    since: Option<DateTime<Utc>>,
    reason: Option<String>,
) -> Result<RemoteLifecycle, StorageError> {
    let since_ts = since.map(timestamp_from_db);
    match (state, since_ts, reason) {
        ("absent", None, None) => Ok(RemoteLifecycle::Absent),
        ("draft", None, None) => Ok(RemoteLifecycle::Draft),
        ("submitted", Some(at), None) => Ok(RemoteLifecycle::Submitted { at }),
        ("in_review", Some(since), None) => Ok(RemoteLifecycle::InReview { since }),
        ("live", Some(since), None) => Ok(RemoteLifecycle::Live { since }),
        ("rejected", Some(at), reason) => Ok(RemoteLifecycle::Rejected { at, reason }),
        ("withdrawn", Some(at), None) => Ok(RemoteLifecycle::Withdrawn { at }),
        (state, since, reason) => Err(StorageError::CorruptRow {
            reason: format!("inconsistent lifecycle columns ({state:?}, {since:?}, {reason:?})"),
        }),
    }
}

struct MismatchColumns {
    field: &'static str,
    class: &'static str,
    observed_in: Option<&'static str>,
    limit_observed: Option<i32>,
}

impl MismatchColumns {
    fn encode(mismatch: &FieldMismatch) -> Result<Self, StorageError> {
        let (class, observed_in, limit_observed) = match &mismatch.class {
            MismatchClass::Normalised => ("normalised", None, None),
            MismatchClass::Truncated { limit_observed } => (
                "truncated",
                None,
                Some(
                    i32::try_from(*limit_observed).map_err(|_| StorageError::Inconsistent {
                        reason: format!("observed limit {limit_observed} exceeds the column range"),
                    })?,
                ),
            ),
            MismatchClass::Missing => ("missing", None, None),
            MismatchClass::WrongField { observed_in } => {
                ("wrong_field", Some(field_key_to_db(*observed_in)), None)
            }
            MismatchClass::Unexpected => ("unexpected", None, None),
        };
        Ok(Self {
            field: field_key_to_db(mismatch.field),
            class,
            observed_in,
            limit_observed,
        })
    }
}

fn mismatch_from_db(row: &MismatchRow) -> Result<FieldMismatch, StorageError> {
    let class = match (
        row.class.as_str(),
        row.observed_in.as_deref(),
        row.limit_observed,
    ) {
        ("normalised", None, None) => MismatchClass::Normalised,
        ("truncated", None, Some(limit)) => MismatchClass::Truncated {
            limit_observed: usize::try_from(limit).map_err(|_| StorageError::CorruptRow {
                reason: format!("negative observed limit {limit}"),
            })?,
        },
        ("missing", None, None) => MismatchClass::Missing,
        ("wrong_field", Some(observed), None) => MismatchClass::WrongField {
            observed_in: field_key_from_db(observed)?,
        },
        ("unexpected", None, None) => MismatchClass::Unexpected,
        (class, observed, limit) => {
            return Err(StorageError::CorruptRow {
                reason: format!(
                    "inconsistent mismatch columns ({class:?}, {observed:?}, {limit:?})"
                ),
            })
        }
    };
    Ok(FieldMismatch {
        field: field_key_from_db(&row.field)?,
        class,
    })
}

const fn field_key_to_db(field: FieldKey) -> &'static str {
    match field {
        FieldKey::Title => "title",
        FieldKey::Description => "description",
        FieldKey::Price => "price",
        FieldKey::Taxonomy => "taxonomy",
        FieldKey::Grades => "grades",
        FieldKey::Files => "files",
    }
}

fn field_key_from_db(raw: &str) -> Result<FieldKey, StorageError> {
    match raw {
        "title" => Ok(FieldKey::Title),
        "description" => Ok(FieldKey::Description),
        "price" => Ok(FieldKey::Price),
        "taxonomy" => Ok(FieldKey::Taxonomy),
        "grades" => Ok(FieldKey::Grades),
        "files" => Ok(FieldKey::Files),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown field key {other:?}"),
        }),
    }
}

const fn policy_to_db(policy: FieldPolicy) -> &'static str {
    match policy {
        FieldPolicy::Managed => "managed",
        FieldPolicy::Frozen => "frozen",
        FieldPolicy::Propose => "propose",
    }
}

fn policy_from_db(raw: &str) -> Result<FieldPolicy, StorageError> {
    match raw {
        "managed" => Ok(FieldPolicy::Managed),
        "frozen" => Ok(FieldPolicy::Frozen),
        "propose" => Ok(FieldPolicy::Propose),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown field policy {other:?}"),
        }),
    }
}

const fn publish_to_db(mode: PublishMode) -> &'static str {
    match mode {
        PublishMode::DryRun => "dry_run",
        PublishMode::Propose => "propose",
        PublishMode::Publish => "publish",
    }
}

fn publish_from_db(raw: &str) -> Result<PublishMode, StorageError> {
    match raw {
        "dry_run" => Ok(PublishMode::DryRun),
        "propose" => Ok(PublishMode::Propose),
        "publish" => Ok(PublishMode::Publish),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown publish mode {other:?}"),
        }),
    }
}

const fn sever_to_db(cause: SeverCause) -> &'static str {
    match cause {
        SeverCause::RemovedByMarketplace => "removed_by_marketplace",
        SeverCause::RemovedBySeller => "removed_by_seller",
        SeverCause::NotFoundOnVerify => "not_found_on_verify",
    }
}

fn sever_from_db(raw: &str) -> Result<SeverCause, StorageError> {
    match raw {
        "removed_by_marketplace" => Ok(SeverCause::RemovedByMarketplace),
        "removed_by_seller" => Ok(SeverCause::RemovedBySeller),
        "not_found_on_verify" => Ok(SeverCause::NotFoundOnVerify),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown sever cause {other:?}"),
        }),
    }
}

struct MappingRow {
    org_id: uuid::Uuid,
    id: uuid::Uuid,
    product_id: uuid::Uuid,
    inventory: String,
    binding_state: String,
    remote_id_kind: Option<String>,
    remote_url: Option<String>,
    remote_numeric_id: Option<i64>,
    binding_attempt: Option<uuid::Uuid>,
    binding_marker: Option<String>,
    first_seen_at: Option<DateTime<Utc>>,
    ambiguous_since: Option<DateTime<Utc>>,
    severed_at: Option<DateTime<Utc>>,
    sever_cause: Option<String>,
    verify_state: String,
    verified_at: Option<DateTime<Utc>>,
    verify_stale_since: Option<DateTime<Utc>>,
    normaliser_version: i32,
    policy_title: String,
    policy_description: String,
    policy_price: String,
    policy_taxonomy: String,
    policy_grades: String,
    policy_files: String,
    price_rule_kind: String,
    price_rate_micros: Option<i64>,
    price_rounding: Option<String>,
    price_explicit_kind: Option<String>,
    price_explicit_minor_units: Option<i64>,
    price_explicit_currency: Option<String>,
    publish_mode: String,
    lifecycle_state: String,
    lifecycle_since: Option<DateTime<Utc>>,
    lifecycle_reason: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

struct MismatchRow {
    field: String,
    class: String,
    observed_in: Option<String>,
    limit_observed: Option<i32>,
}

struct CandidateRow {
    kind: String,
    url: Option<String>,
    numeric_id: Option<i64>,
}

fn decode_verification(
    row: &MappingRow,
    mismatches: &[MismatchRow],
) -> Result<Verification, StorageError> {
    match (
        row.verify_state.as_str(),
        row.verified_at,
        row.verify_stale_since,
    ) {
        ("stale", None, Some(since)) => Ok(Verification::Stale {
            since: timestamp_from_db(since),
        }),
        ("clean", Some(at), None) => Ok(Verification::Clean {
            at: timestamp_from_db(at),
        }),
        ("mismatched", Some(at), None) => {
            let mut decoded = mismatches.iter().map(mismatch_from_db);
            let first = decoded
                .next()
                .transpose()?
                .ok_or_else(|| StorageError::CorruptRow {
                    reason: "mismatched verification without a field_mismatch row".to_owned(),
                })?;
            let rest = decoded.collect::<Result<Vec<_>, _>>()?;
            Ok(Verification::Mismatched {
                at: timestamp_from_db(at),
                first,
                rest,
            })
        }
        (state, at, since) => Err(StorageError::CorruptRow {
            reason: format!("inconsistent verify columns ({state:?}, {at:?}, {since:?})"),
        }),
    }
}

fn decode_binding(
    row: &MappingRow,
    mismatches: &[MismatchRow],
    candidates: Vec<CandidateRow>,
) -> Result<Binding, StorageError> {
    match row.binding_state.as_str() {
        "unbound" => Ok(Binding::Unbound),
        "creating" => Ok(Binding::Creating {
            attempt: AttemptId(uuid_from_db(row.binding_attempt.ok_or_else(|| {
                StorageError::CorruptRow {
                    reason: "creating binding without an attempt".to_owned(),
                }
            })?)),
            marker: row.binding_marker.clone().map(CorrelationMarker),
        }),
        "bound" => Ok(Binding::Bound {
            id: remote_id_from_db(
                row.remote_id_kind
                    .as_deref()
                    .ok_or_else(|| StorageError::CorruptRow {
                        reason: "bound binding without a remote id kind".to_owned(),
                    })?,
                row.remote_url.clone(),
                row.remote_numeric_id,
            )?,
            first_seen: timestamp_from_db(row.first_seen_at.ok_or_else(|| {
                StorageError::CorruptRow {
                    reason: "bound binding without first_seen_at".to_owned(),
                }
            })?),
            verified: decode_verification(row, mismatches)?,
        }),
        "ambiguous_create" => Ok(Binding::AmbiguousCreate {
            attempt: AttemptId(uuid_from_db(row.binding_attempt.ok_or_else(|| {
                StorageError::CorruptRow {
                    reason: "ambiguous create without an attempt".to_owned(),
                }
            })?)),
            candidates: candidates
                .into_iter()
                .map(|c| remote_id_from_db(&c.kind, c.url, c.numeric_id))
                .collect::<Result<Vec<_>, _>>()?,
            since: timestamp_from_db(row.ambiguous_since.ok_or_else(|| {
                StorageError::CorruptRow {
                    reason: "ambiguous create without a since instant".to_owned(),
                }
            })?),
        }),
        "severed" => Ok(Binding::Severed {
            was: remote_id_from_db(
                row.remote_id_kind
                    .as_deref()
                    .ok_or_else(|| StorageError::CorruptRow {
                        reason: "severed binding without a remote id kind".to_owned(),
                    })?,
                row.remote_url.clone(),
                row.remote_numeric_id,
            )?,
            noticed: timestamp_from_db(row.severed_at.ok_or_else(|| StorageError::CorruptRow {
                reason: "severed binding without severed_at".to_owned(),
            })?),
            cause: sever_from_db(row.sever_cause.as_deref().ok_or_else(|| {
                StorageError::CorruptRow {
                    reason: "severed binding without a cause".to_owned(),
                }
            })?)?,
        }),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown binding state {other:?}"),
        }),
    }
}

fn decode_price_rule(row: &MappingRow) -> Result<PriceRule, StorageError> {
    match row.price_rule_kind.as_str() {
        "converted" => {
            let rate_micros = row
                .price_rate_micros
                .ok_or_else(|| StorageError::CorruptRow {
                    reason: "converted price rule without a rate".to_owned(),
                })?;
            let rounding = match row.price_rounding.as_deref() {
                Some("nearest") => Rounding::Nearest,
                Some("up_to_charm") => Rounding::UpToCharm,
                other => {
                    return Err(StorageError::CorruptRow {
                        reason: format!("unknown rounding {other:?}"),
                    })
                }
            };
            Ok(PriceRule::Converted {
                rate_micros,
                rounding,
            })
        }
        "explicit" => {
            let kind =
                row.price_explicit_kind
                    .as_deref()
                    .ok_or_else(|| StorageError::CorruptRow {
                        reason: "explicit price rule without a price kind".to_owned(),
                    })?;
            let intent: PriceIntent = if kind == PRICE_KIND_FREE || kind == PRICE_KIND_PAID {
                price_from_db(
                    kind,
                    row.price_explicit_minor_units,
                    row.price_explicit_currency.clone(),
                )?
            } else {
                return Err(StorageError::CorruptRow {
                    reason: format!("unknown explicit price kind {kind:?}"),
                });
            };
            Ok(PriceRule::Explicit(intent))
        }
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown price rule kind {other:?}"),
        }),
    }
}

fn decode_mapping(
    row: &MappingRow,
    mismatches: &[MismatchRow],
    candidates: Vec<CandidateRow>,
) -> Result<MappingRecord, StorageError> {
    let binding = decode_binding(row, mismatches, candidates)?;
    let inventory: InventoryId = inventory_from_db(&row.inventory)?;
    let policies = FieldPolicies {
        title: policy_from_db(&row.policy_title)?,
        description: policy_from_db(&row.policy_description)?,
        price: policy_from_db(&row.policy_price)?,
        taxonomy: policy_from_db(&row.policy_taxonomy)?,
        grades: policy_from_db(&row.policy_grades)?,
        files: policy_from_db(&row.policy_files)?,
    };
    let price_rule = decode_price_rule(row)?;
    let publish = publish_from_db(&row.publish_mode)?;
    let lifecycle = lifecycle_from_db(
        &row.lifecycle_state,
        row.lifecycle_since,
        row.lifecycle_reason.clone(),
    )?;
    let normaliser_version =
        u32::try_from(row.normaliser_version).map_err(|_| StorageError::CorruptRow {
            reason: format!("negative normaliser version {}", row.normaliser_version),
        })?;
    Ok(MappingRecord {
        mapping: Mapping {
            id: MappingId(uuid_from_db(row.id)),
            org: OrgId(uuid_from_db(row.org_id)),
            product: ProductId(uuid_from_db(row.product_id)),
            inventory,
            binding,
            policies,
            price_rule,
            publish,
            lifecycle,
        },
        normaliser_version,
        created_at: timestamp_from_db(row.created_at),
        updated_at: timestamp_from_db(row.updated_at),
    })
}

/// The flat row the client's product-by-inventory table joins on: one query,
/// no aggregate hydration, because the table renders state labels rather
/// than the mapping aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingHead {
    pub id: MappingId,
    pub product: ProductId,
    pub inventory: InventoryId,
    pub binding_state: String,
    pub lifecycle_state: String,
    pub updated_at: Timestamp,
}

impl MappingRepo {
    pub async fn list_heads(&self, org: OrgId) -> Result<Vec<MappingHead>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT id, product_id, inventory, binding_state, lifecycle_state, updated_at \
             FROM mapping WHERE org_id = $1 ORDER BY product_id, inventory",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(MappingHead {
                    id: MappingId(uuid_from_db(row.id)),
                    product: ProductId(uuid_from_db(row.product_id)),
                    inventory: crate::codec::inventory_from_db(&row.inventory)?,
                    binding_state: row.binding_state,
                    lifecycle_state: row.lifecycle_state,
                    updated_at: timestamp_from_db(row.updated_at),
                })
            })
            .collect()
    }
}

/// The mapping, attempt and instant one batch of losses is recorded under;
/// the losses themselves vary per value and travel separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LossScope {
    pub org: OrgId,
    pub mapping: MappingId,
    pub attempt: AttemptId,
    pub at: Timestamp,
}

/// One recorded loss as it reads back: what kind it was, which axis it names
/// where it names one, and the variant's own payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedLoss {
    pub attempt: AttemptId,
    pub kind: LossKind,
    pub axis: Option<TermKind>,
    pub detail: serde_json::Value,
    pub recorded_at: Timestamp,
}

impl MappingRepo {
    /// Records what one projection attempt could not carry.
    ///
    /// Insert-only and keyed on the attempt, which is what buys the engine's
    /// no-delete rule: a re-projection writes a new attempt's rows rather than
    /// replacing the previous ones, so every attempt's losses stay queryable
    /// and no delete grant is ever needed.
    pub async fn record_losses(
        &self,
        scope: LossScope,
        losses: &[Loss],
    ) -> Result<u64, StorageError> {
        let LossScope {
            org,
            mapping,
            attempt,
            at,
        } = scope;
        if losses.is_empty() {
            return Ok(0);
        }
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let mut written = 0;
        for (position, loss) in losses.iter().enumerate() {
            let position = i32::try_from(position).map_err(|_| StorageError::Inconsistent {
                reason: "a projection cannot lose more values than an int can count".to_owned(),
            })?;
            written += sqlx::query!(
                "INSERT INTO mapping_loss \
                 (org_id, mapping_id, attempt, position, kind, axis, detail, recorded_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                uuid_to_db(org.0),
                uuid_to_db(mapping.0),
                uuid_to_db(attempt.0),
                position,
                loss.kind().as_str(),
                loss_axis(loss).map(term_kind_to_db),
                loss_detail(loss),
                timestamp_to_db(at)?,
            )
            .execute(&mut *tx)
            .await?
            .rows_affected();
        }
        tx.commit().await?;
        Ok(written)
    }

    /// What one mapping has lost, newest attempt first, so a decision surface
    /// can show the seller what this listing gives up whatever they pick.
    pub async fn losses(
        &self,
        org: OrgId,
        mapping: MappingId,
    ) -> Result<Vec<RecordedLoss>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT attempt, kind, axis, detail, recorded_at FROM mapping_loss \
             WHERE org_id = $1 AND mapping_id = $2 \
             ORDER BY recorded_at DESC, attempt, position",
            uuid_to_db(org.0),
            uuid_to_db(mapping.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(RecordedLoss {
                    attempt: AttemptId(uuid_from_db(row.attempt)),
                    kind: loss_kind_from_db(&row.kind)?,
                    axis: row.axis.as_deref().map(term_kind_from_db).transpose()?,
                    detail: row.detail,
                    recorded_at: timestamp_from_db(row.recorded_at),
                })
            })
            .collect()
    }
}

/// Only a no-target-field loss names an axis; the others are about values
/// within an axis the target does bind.
const fn loss_axis(loss: &Loss) -> Option<TermKind> {
    match loss {
        Loss::NoTargetField { axis, .. } => Some(*axis),
        Loss::Broadened { .. } | Loss::Collapsed { .. } | Loss::Elected { .. } => None,
    }
}

fn path_json(path: &VocabularyPath) -> serde_json::Value {
    serde_json::json!({ "segments": path.segments, "native_id": path.native_id })
}

fn loss_detail(loss: &Loss) -> serde_json::Value {
    match loss {
        Loss::Broadened { to, dropped } => serde_json::json!({
            "to": path_json(to),
            "dropped": dropped.iter().map(|term| hex_uuid(term.0)).collect::<Vec<_>>(),
        }),
        Loss::NoTargetField { value, .. } => serde_json::json!({ "value": path_json(value) }),
        Loss::Collapsed { to, from } => serde_json::json!({
            "to": path_json(to),
            "from": from.iter().map(path_json).collect::<Vec<_>>(),
        }),
        Loss::Elected { kept, dropped, by } => serde_json::json!({
            "kept": kept.iter().map(path_json).collect::<Vec<_>>(),
            "dropped": dropped.iter().map(path_json).collect::<Vec<_>>(),
            "by": match by {
                Decider::Imported { source } => serde_json::json!({ "imported": source }),
                Decider::Human { user, org } => serde_json::json!({
                    "user": hex_uuid(user.0),
                    "org": hex_uuid(org.0),
                }),
            },
        }),
    }
}

fn hex_uuid(id: tam_types::Uuid) -> String {
    uuid::Uuid::from_bytes(id.0).to_string()
}

fn loss_kind_from_db(raw: &str) -> Result<LossKind, StorageError> {
    match raw {
        "broadened" => Ok(LossKind::Broadened),
        "no_target_field" => Ok(LossKind::NoTargetField),
        "collapsed" => Ok(LossKind::Collapsed),
        "elected" => Ok(LossKind::Elected),
        other => Err(StorageError::Inconsistent {
            reason: format!("unknown loss kind {other}"),
        }),
    }
}
