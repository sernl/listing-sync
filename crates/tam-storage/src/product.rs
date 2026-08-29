//! The catalogue aggregate: a product with its files, blobs, taxonomy
//! assignments and grade declaration, written and read as one unit inside one
//! pinned transaction. The deferred `assert_product_has_payload` trigger
//! re-states `PayloadSet`'s non-emptiness at commit, so a partial write
//! cannot leave a payload-less product behind.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tam_domain::{
    AgeInterval, CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration,
    TermKind, VocabularyId, VocabularyPath,
};
use tam_types::{
    CanonicalTermId, FileId, FileRole, ImportedTerm, OrgId, PayloadSet, PriceIntent, ProductFile,
    ProductId, Timestamp, Title,
};

use crate::codec::{
    copy_format_from_db, copy_format_to_db, file_kind_from_db, file_kind_to_db, file_role_from_db,
    file_role_to_db, hash_from_db, hash_hex, hash_to_db, inventory_from_db, inventory_to_db,
    price_from_db, scan_from_db, term_kind_from_db, term_kind_to_db, timestamp_from_db,
    timestamp_to_db, uuid_from_db, uuid_to_db, PriceColumns, ScanColumns,
};
use crate::{pin_org, StorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductRecord {
    pub product: CanonicalProduct,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductSummary {
    pub org: OrgId,
    pub id: ProductId,
    pub title: Title,
    pub price: PriceIntent,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub struct ProductRepo {
    pool: PgPool,
}

impl ProductRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(
        &self,
        org: OrgId,
        product: &CanonicalProduct,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        if product.org != org {
            return Err(StorageError::OrgMismatch);
        }
        let org_db = uuid_to_db(org.0);
        let product_db = uuid_to_db(product.id.0);
        let at_db = timestamp_to_db(at)?;

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;

        let price = PriceColumns::from_intent(product.price);
        let rights = RightsColumns::encode(&product.rights);
        sqlx::query!(
            "INSERT INTO product \
             (org_id, id, title, body, body_format, price_kind, price_minor_units, \
              price_currency, rights_state, rights_source_inventory, rights_segments, \
              rights_native_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)",
            org_db,
            product_db,
            product.title.0,
            product.body.body,
            copy_format_to_db(product.body.format),
            price.kind,
            price.minor_units,
            price.currency,
            rights.state,
            rights.inventory,
            rights.segments.as_deref(),
            rights.native_id,
            at_db,
        )
        .execute(&mut *tx)
        .await?;

        let write = FileWrite {
            org: org_db,
            product: product_db,
            at: at_db,
        };
        let mut position: i32 = 0;
        for file in product.payload.iter() {
            insert_file(&mut tx, &write, position, FileRole::Payload, file).await?;
            position += 1;
        }
        if let Some(cover) = &product.cover {
            insert_file(&mut tx, &write, position, FileRole::Cover, cover).await?;
            position += 1;
        }
        for preview in &product.previews {
            insert_file(&mut tx, &write, position, FileRole::Preview, preview).await?;
            position += 1;
        }

        for (index, term) in product.subjects.iter().enumerate() {
            let term_position = i32::try_from(index).map_err(|_| StorageError::Inconsistent {
                reason: format!("subject position {index} exceeds the column range"),
            })?;
            sqlx::query!(
                "INSERT INTO product_term (org_id, product_id, term_id, position) \
                 VALUES ($1, $2, $3, $4)",
                org_db,
                product_db,
                uuid_to_db(term.0),
                term_position,
            )
            .execute(&mut *tx)
            .await?;
        }

        insert_grades(&mut tx, org_db, product_db, &product.grades).await?;

        for (index, term) in product.native_residue.iter().enumerate() {
            let position = i32::try_from(index).map_err(|_| StorageError::Inconsistent {
                reason: format!("residue position {index} exceeds the column range"),
            })?;
            sqlx::query!(
                "INSERT INTO native_residue \
                 (org_id, product_id, position, inventory, term_kind, segments, native_id) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                org_db,
                product_db,
                position,
                inventory_to_db(term.inventory),
                term.kind.map(term_kind_to_db),
                &term.segments,
                term.native_id.as_deref(),
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
        id: ProductId,
    ) -> Result<Option<ProductRecord>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let product_db = uuid_to_db(id.0);

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;

        let Some(row) = sqlx::query_as!(
            ProductRow,
            "SELECT org_id, id, title, body, body_format, price_kind, price_minor_units, \
             price_currency, rights_state, rights_source_inventory, rights_segments, \
             rights_native_id, created_at, updated_at \
             FROM product \
             WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
            org_db,
            product_db,
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };

        let files = sqlx::query_as!(
            FileRow,
            "SELECT f.id, f.role, f.kind, f.hash, b.byte_len AS \"byte_len!\", \
             f.scan_state, f.scan_signature, f.scanned_at, f.scan_failure_code \
             FROM product_file f \
             JOIN blob b ON b.org_id = f.org_id AND b.hash = f.hash \
             WHERE f.org_id = $1 AND f.product_id = $2 AND f.deleted_at IS NULL \
             ORDER BY f.position",
            org_db,
            product_db,
        )
        .fetch_all(&mut *tx)
        .await?;

        let terms = sqlx::query!(
            "SELECT term_id FROM product_term \
             WHERE org_id = $1 AND product_id = $2 \
             ORDER BY position",
            org_db,
            product_db,
        )
        .fetch_all(&mut *tx)
        .await?;

        let grade = sqlx::query_as!(
            GradeRow,
            "SELECT source, source_inventory, source_term_kind, \
             derived_low_years, derived_high_years \
             FROM grade_declaration \
             WHERE org_id = $1 AND product_id = $2",
            org_db,
            product_db,
        )
        .fetch_optional(&mut *tx)
        .await?;

        let paths = sqlx::query_as!(
            PathRow,
            "SELECT inventory, term_kind, segments, native_id \
             FROM grade_declaration_path \
             WHERE org_id = $1 AND product_id = $2 \
             ORDER BY position",
            org_db,
            product_db,
        )
        .fetch_all(&mut *tx)
        .await?;

        let residue = sqlx::query_as!(
            ResidueRow,
            "SELECT inventory, term_kind, segments, native_id \
             FROM native_residue \
             WHERE org_id = $1 AND product_id = $2 \
             ORDER BY position",
            org_db,
            product_db,
        )
        .fetch_all(&mut *tx)
        .await?;

        tx.commit().await?;

        let native_residue = residue
            .into_iter()
            .map(decode_residue)
            .collect::<Result<Vec<_>, _>>()?;
        let (payload, cover, previews) = partition_files(files)?;
        let subjects = terms
            .into_iter()
            .map(|row| CanonicalTermId(uuid_from_db(row.term_id)))
            .collect();
        let grade = grade.ok_or_else(|| StorageError::CorruptRow {
            reason: "product without a grade_declaration row".to_owned(),
        })?;
        let grades = decode_grades(&grade, paths)?;
        let rights = rights_from_db(&row)?;
        let price = price_from_db(&row.price_kind, row.price_minor_units, row.price_currency)?;

        Ok(Some(ProductRecord {
            product: CanonicalProduct {
                id: ProductId(uuid_from_db(row.id)),
                org: OrgId(uuid_from_db(row.org_id)),
                title: Title(row.title),
                body: tam_types::ListingCopy {
                    body: row.body,
                    format: copy_format_from_db(&row.body_format)?,
                },
                payload,
                cover,
                previews,
                subjects,
                grades,
                price,
                rights,
                native_residue,
            },
            created_at: timestamp_from_db(row.created_at),
            updated_at: timestamp_from_db(row.updated_at),
        }))
    }

    pub async fn list(&self, org: OrgId) -> Result<Vec<ProductSummary>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query_as!(
            SummaryRow,
            "SELECT org_id, id, title, price_kind, price_minor_units, price_currency, \
             created_at, updated_at \
             FROM product \
             WHERE org_id = $1 AND deleted_at IS NULL \
             ORDER BY created_at, id",
            org_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(ProductSummary {
                    org: OrgId(uuid_from_db(row.org_id)),
                    id: ProductId(uuid_from_db(row.id)),
                    title: Title(row.title),
                    price: price_from_db(
                        &row.price_kind,
                        row.price_minor_units,
                        row.price_currency,
                    )?,
                    created_at: timestamp_from_db(row.created_at),
                    updated_at: timestamp_from_db(row.updated_at),
                })
            })
            .collect()
    }
}

struct FileWrite {
    org: uuid::Uuid,
    product: uuid::Uuid,
    at: DateTime<Utc>,
}

async fn insert_file(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    write: &FileWrite,
    position: i32,
    slot: FileRole,
    file: &ProductFile,
) -> Result<(), StorageError> {
    let FileWrite {
        org: org_db,
        product: product_db,
        at: at_db,
    } = *write;
    if file.role != slot {
        return Err(StorageError::Inconsistent {
            reason: format!(
                "file {:?} carries role {:?} but sits in the {:?} slot",
                file.id, file.role, slot
            ),
        });
    }
    let hash = hash_to_db(file.hash);
    let byte_len = i64::try_from(file.byte_len).map_err(|_| StorageError::Inconsistent {
        reason: format!("byte length {} exceeds the column range", file.byte_len),
    })?;
    let object_key = format!("blob/{}", hash_hex(file.hash));

    // dek_key_version 0 is the not-yet-encrypted sentinel; M1f assigns real
    // envelope-encryption versions when the file pipeline lands.
    sqlx::query!(
        "INSERT INTO blob (org_id, hash, byte_len, object_key, dek_key_version, first_seen_at) \
         VALUES ($1, $2, $3, $4, 0, $5) \
         ON CONFLICT (org_id, hash) DO NOTHING",
        org_db,
        hash,
        byte_len,
        object_key,
        at_db,
    )
    .execute(&mut **tx)
    .await?;

    let stored_len = sqlx::query_scalar!(
        "SELECT byte_len FROM blob WHERE org_id = $1 AND hash = $2",
        org_db,
        hash,
    )
    .fetch_one(&mut **tx)
    .await?;
    if stored_len != byte_len {
        return Err(StorageError::Inconsistent {
            reason: format!(
                "blob byte_len {stored_len} disagrees with file byte_len {byte_len} for one hash"
            ),
        });
    }

    let scan = ScanColumns::from_outcome(&file.scan)?;
    sqlx::query!(
        "INSERT INTO product_file \
         (org_id, id, product_id, position, role, kind, hash, \
          scan_state, scan_signature, scanned_at, scan_failure_code, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        org_db,
        uuid_to_db(file.id.0),
        product_db,
        position,
        file_role_to_db(file.role),
        file_kind_to_db(file.kind),
        hash,
        scan.state,
        scan.signature,
        scan.scanned_at,
        scan.failure_code,
        at_db,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_grades(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    product_db: uuid::Uuid,
    grades: &GradeDeclaration,
) -> Result<(), StorageError> {
    let (source, source_inventory, source_term_kind) = match grades.source {
        DeclarationSource::Seller => ("seller", None, None),
        DeclarationSource::Imported {
            vocabulary: VocabularyId(inventory, kind),
        } => (
            "imported",
            Some(inventory_to_db(inventory)),
            Some(term_kind_to_db(kind)),
        ),
    };
    let (low, high) = match grades.derived {
        None => (None, None),
        Some(interval) => (
            Some(i16::from(interval.low_years())),
            Some(i16::from(interval.high_years())),
        ),
    };
    sqlx::query!(
        "INSERT INTO grade_declaration \
         (org_id, product_id, source, source_inventory, source_term_kind, \
          derived_low_years, derived_high_years) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        org_db,
        product_db,
        source,
        source_inventory,
        source_term_kind,
        low,
        high,
    )
    .execute(&mut **tx)
    .await?;

    for (index, path) in grades.raw.iter().enumerate() {
        let position = i32::try_from(index).map_err(|_| StorageError::Inconsistent {
            reason: format!("grade path position {index} exceeds the column range"),
        })?;
        let VocabularyId(inventory, kind) = path.vocabulary;
        sqlx::query!(
            "INSERT INTO grade_declaration_path \
             (org_id, product_id, position, inventory, term_kind, segments, native_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            org_db,
            product_db,
            position,
            inventory_to_db(inventory),
            term_kind_to_db(kind),
            &path.segments,
            path.native_id.as_deref(),
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

struct ProductRow {
    org_id: uuid::Uuid,
    id: uuid::Uuid,
    title: String,
    body: String,
    body_format: String,
    price_kind: String,
    price_minor_units: Option<i64>,
    price_currency: Option<String>,
    rights_state: String,
    rights_source_inventory: Option<String>,
    rights_segments: Option<Vec<String>>,
    rights_native_id: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// The four columns a rights declaration occupies, so the insert and the
/// `product_rights_total` CHECK cannot disagree about which combination is
/// representable.
struct RightsColumns {
    state: &'static str,
    inventory: Option<String>,
    segments: Option<Vec<String>>,
    native_id: Option<String>,
}

impl RightsColumns {
    fn encode(rights: &RightsDeclaration) -> Self {
        match rights {
            RightsDeclaration::Unstated => Self {
                state: "unstated",
                inventory: None,
                segments: None,
                native_id: None,
            },
            RightsDeclaration::Declared { source } => Self {
                state: "declared",
                inventory: Some(inventory_to_db(source.vocabulary.0).to_owned()),
                segments: Some(source.segments.clone()),
                native_id: source.native_id.clone(),
            },
        }
    }
}

/// The kind is not stored: a rights declaration is a licence by construction,
/// and a column that could disagree with that is a column that will.
fn rights_from_db(row: &ProductRow) -> Result<RightsDeclaration, StorageError> {
    match (
        row.rights_state.as_str(),
        row.rights_source_inventory.as_deref(),
        row.rights_segments.as_ref(),
    ) {
        ("unstated", None, None) => Ok(RightsDeclaration::Unstated),
        ("declared", Some(inventory), Some(segments)) => Ok(RightsDeclaration::Declared {
            source: VocabularyPath {
                vocabulary: VocabularyId(inventory_from_db(inventory)?, TermKind::Licence),
                segments: segments.clone(),
                native_id: row.rights_native_id.clone(),
            },
        }),
        (state, inventory, segments) => Err(StorageError::CorruptRow {
            reason: format!("inconsistent rights columns ({state:?}, {inventory:?}, {segments:?})"),
        }),
    }
}

struct SummaryRow {
    org_id: uuid::Uuid,
    id: uuid::Uuid,
    title: String,
    price_kind: String,
    price_minor_units: Option<i64>,
    price_currency: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

struct FileRow {
    id: uuid::Uuid,
    role: String,
    kind: String,
    hash: Vec<u8>,
    byte_len: i64,
    scan_state: String,
    scan_signature: Option<String>,
    scanned_at: Option<DateTime<Utc>>,
    scan_failure_code: Option<String>,
}

struct GradeRow {
    source: String,
    source_inventory: Option<String>,
    source_term_kind: Option<String>,
    derived_low_years: Option<i16>,
    derived_high_years: Option<i16>,
}

struct PathRow {
    inventory: String,
    term_kind: String,
    segments: Vec<String>,
    native_id: Option<String>,
}

/// The kind is nullable here and not on `PathRow`: a grade declaration is a
/// phase by construction, while a residue value's axis is exactly what is not
/// known about it.
struct ResidueRow {
    inventory: String,
    term_kind: Option<String>,
    segments: Vec<String>,
    native_id: Option<String>,
}

fn decode_residue(row: ResidueRow) -> Result<ImportedTerm, StorageError> {
    Ok(ImportedTerm {
        inventory: inventory_from_db(&row.inventory)?,
        kind: row
            .term_kind
            .as_deref()
            .map(term_kind_from_db)
            .transpose()?,
        segments: row.segments,
        native_id: row.native_id,
    })
}

fn decode_file(row: FileRow) -> Result<(FileRole, ProductFile), StorageError> {
    let role = file_role_from_db(&row.role)?;
    Ok((
        role,
        ProductFile {
            id: FileId(uuid_from_db(row.id)),
            role,
            kind: file_kind_from_db(&row.kind)?,
            hash: hash_from_db(&row.hash)?,
            byte_len: u64::try_from(row.byte_len).map_err(|_| StorageError::CorruptRow {
                reason: format!("negative blob byte_len {}", row.byte_len),
            })?,
            scan: scan_from_db(
                &row.scan_state,
                row.scan_signature,
                row.scanned_at,
                row.scan_failure_code.as_deref(),
            )?,
        },
    ))
}

fn partition_files(
    rows: Vec<FileRow>,
) -> Result<(PayloadSet, Option<ProductFile>, Vec<ProductFile>), StorageError> {
    let mut payload = Vec::new();
    let mut cover = None;
    let mut previews = Vec::new();
    for row in rows {
        let (role, file) = decode_file(row)?;
        match role {
            FileRole::Payload => payload.push(file),
            FileRole::Preview => previews.push(file),
            FileRole::Cover => {
                if cover.replace(file).is_some() {
                    return Err(StorageError::CorruptRow {
                        reason: "more than one live cover file".to_owned(),
                    });
                }
            }
        }
    }
    let mut payload = payload.into_iter();
    let head = payload.next().ok_or_else(|| StorageError::CorruptRow {
        reason: "product without a live payload file".to_owned(),
    })?;
    Ok((PayloadSet::new(head, payload.collect()), cover, previews))
}

fn decode_grades(row: &GradeRow, paths: Vec<PathRow>) -> Result<GradeDeclaration, StorageError> {
    let source = match (
        row.source.as_str(),
        row.source_inventory.as_deref(),
        row.source_term_kind.as_deref(),
    ) {
        ("seller", None, None) => DeclarationSource::Seller,
        ("imported", Some(inventory), Some(kind)) => DeclarationSource::Imported {
            vocabulary: VocabularyId(inventory_from_db(inventory)?, term_kind_from_db(kind)?),
        },
        (source, inventory, kind) => {
            return Err(StorageError::CorruptRow {
                reason: format!(
                    "inconsistent grade source columns ({source:?}, {inventory:?}, {kind:?})"
                ),
            })
        }
    };
    let derived = match (row.derived_low_years, row.derived_high_years) {
        (None, None) => None,
        (Some(low), Some(high)) => {
            let low = u8::try_from(low).map_err(|_| StorageError::CorruptRow {
                reason: format!("derived low years {low} outside u8"),
            })?;
            let high = u8::try_from(high).map_err(|_| StorageError::CorruptRow {
                reason: format!("derived high years {high} outside u8"),
            })?;
            Some(
                AgeInterval::new(low, high).map_err(|_| StorageError::CorruptRow {
                    reason: format!("inverted derived interval {low}..{high}"),
                })?,
            )
        }
        (low, high) => {
            return Err(StorageError::CorruptRow {
                reason: format!("half-set derived interval ({low:?}, {high:?})"),
            })
        }
    };
    let raw = paths
        .into_iter()
        .map(|path| {
            Ok(VocabularyPath {
                vocabulary: VocabularyId(
                    inventory_from_db(&path.inventory)?,
                    term_kind_from_db(&path.term_kind)?,
                ),
                segments: path.segments,
                native_id: path.native_id,
            })
        })
        .collect::<Result<Vec<_>, StorageError>>()?;
    Ok(GradeDeclaration {
        source,
        raw,
        derived,
    })
}

impl ProductRepo {
    /// The catalogue page: keyset on `(created_at, id)` strictly above the
    /// cursor, oldest first, matching the unpaginated listing's order.
    pub async fn list_page(
        &self,
        org: OrgId,
        cursor: Option<crate::job_reads::LedgerCursor>,
        limit: i64,
    ) -> Result<Vec<ProductSummary>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let (cursor_at, cursor_id) = match cursor {
            Some(cursor) => (
                Some(timestamp_to_db(cursor.created_at)?),
                Some(uuid_to_db(cursor.id)),
            ),
            None => (None, None),
        };
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query_as!(
            SummaryRow,
            "SELECT org_id, id, title, price_kind, price_minor_units, price_currency, \
             created_at, updated_at \
             FROM product \
             WHERE org_id = $1 AND deleted_at IS NULL \
               AND ($2::timestamptz IS NULL OR (created_at, id) > ($2, $3)) \
             ORDER BY created_at, id LIMIT $4",
            org_db,
            cursor_at,
            cursor_id,
            limit,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(ProductSummary {
                    org: OrgId(uuid_from_db(row.org_id)),
                    id: ProductId(uuid_from_db(row.id)),
                    title: Title(row.title),
                    price: price_from_db(
                        &row.price_kind,
                        row.price_minor_units,
                        row.price_currency,
                    )?,
                    created_at: timestamp_from_db(row.created_at),
                    updated_at: timestamp_from_db(row.updated_at),
                })
            })
            .collect()
    }
}
