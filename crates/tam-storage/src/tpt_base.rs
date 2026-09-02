//! The TPT-base fields of a product, beside the product rather than inside it.
//!
//! `product` already stores the name, the description with its declared
//! format, the price, the payload files and the verbatim grade declaration.
//! Everything else TPT's create form asks for lands here, in the sidecar
//! migration 0040 declares, keyed on the product's own key.
//!
//! One row per product, and a product with no row is one authored before this
//! table existed or through a path that does not carry these fields. Every
//! read is therefore an outer join at the caller and the absence is a fact
//! rather than a fault: [`TptBaseRepo::get`] answers `None` and means it.
//!
//! Two encodings are deliberate. The picker arrays hold TPT facet slugs
//! verbatim, because that is what the seller chose and what the wire takes;
//! the projection onto another marketplace goes through the crosswalk
//! relation and never through this table. And the wire ids of the closed
//! listbox vocabularies are stored as the small integers they are, decoded
//! back through the domain's own `from_wire_id` so a row outside the set names
//! itself as corruption instead of arriving as a plausible neighbour.
//!
//! Folding this into `product` is a scheduled step after the engine driver
//! split, not a permanent shape.

use sqlx::PgPool;
use tam_domain::product::{
    AnswerKey, CategoryGroup, CopyrightDeclaration, DetailGroup, FacetSlug, ListingStatus,
    StandardAlignment, StandardsFramework, TaxCode, TeachingDuration, ThumbnailMode, UploadRef,
};
use tam_types::{OrgId, ProductId, Timestamp};

use crate::codec::{timestamp_to_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// One product's TPT-base fields, as the sidecar holds them.
///
/// Domain types rather than raw columns, so a value that cannot be a member of
/// its vocabulary cannot be constructed here and a stored row outside the set
/// is refused on the way out rather than carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TptBaseRecord {
    pub thumbnail_mode: ThumbnailMode,
    /// `data[ItemDigital][thumb1..thumb4]`, first slot first.
    pub thumbnails: Vec<UploadRef>,
    pub video_preview: Option<UploadRef>,
    /// `data[Item][license_price]`. Stored as the seller set it and never
    /// recomputed: TPT's ninety percent is the form's pre-fill, so deriving it
    /// on a sync would overwrite their own figure (D6).
    pub additional_licence_minor_units: Option<i64>,
    pub bundle_discount_minor_units: Option<i64>,
    /// Never defaulted: designating a tax code is the seller's under TPT's
    /// terms, so `None` is a listing that has not chosen one (D7).
    pub tax_code: Option<TaxCode>,
    pub categories: CategoryGroup,
    pub standards: Vec<StandardAlignment>,
    pub details: DetailGroup,
    /// `None` is a product that cannot be submitted rather than one attested
    /// by default. TPT's own form arrives with value 1 pre-selected and ours
    /// must not.
    pub copyright: Option<CopyrightDeclaration>,
    pub status: ListingStatus,
}

/// The `jsonb` column, built and read by hand.
///
/// `serde_json` is in this crate's graph and `serde`'s derive is not, and a
/// three-field row is not worth a dependency change, which is founder-gated.
/// The column stores TPT's jurisdiction id where the domain holds the
/// framework it names, and [`read_standards`] is where an id naming no
/// framework the create form offers is refused rather than dropped.
fn write_standards(alignments: &[StandardAlignment]) -> serde_json::Value {
    serde_json::Value::Array(
        alignments
            .iter()
            .map(|alignment| {
                serde_json::json!({
                    "framework": alignment.framework.jurisdiction_id(),
                    "code": alignment.code,
                    "tpt_node_id": alignment.tpt_node_id,
                })
            })
            .collect(),
    )
}

fn read_standards(stored: &serde_json::Value) -> Result<Vec<StandardAlignment>, StorageError> {
    let corrupt = |reason: String| StorageError::CorruptRow { reason };
    let held = stored
        .as_array()
        .ok_or_else(|| corrupt("the standards column is not an array".to_owned()))?;
    held.iter()
        .map(|row| {
            let jurisdiction = row
                .get("framework")
                .and_then(serde_json::Value::as_u64)
                .and_then(|id| u32::try_from(id).ok())
                .ok_or_else(|| corrupt("a standards row names no jurisdiction".to_owned()))?;
            let framework =
                StandardsFramework::from_jurisdiction_id(jurisdiction).ok_or_else(|| {
                    corrupt(format!(
                        "jurisdiction {jurisdiction} is not one the create form offers"
                    ))
                })?;
            let code = row
                .get("code")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| corrupt("a standards row carries no code".to_owned()))?;
            Ok(StandardAlignment {
                framework,
                code: code.to_owned(),
                tpt_node_id: row.get("tpt_node_id").and_then(serde_json::Value::as_u64),
            })
        })
        .collect()
}

pub struct TptBaseRepo {
    pool: PgPool,
}

impl TptBaseRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Writes this product's TPT-base fields, replacing whatever was there.
    ///
    /// An upsert rather than an insert, because the create and the edit write
    /// the same row: the form is one model and TPT's own create and edit post
    /// a near-identical body, so a second write of a product is an edit rather
    /// than a conflict.
    ///
    /// The whole row is replaced rather than merged. A field the caller left
    /// unset is unset, which is what an edit that cleared a control means; a
    /// merge would make clearing a control impossible to express.
    pub async fn upsert(
        &self,
        org: OrgId,
        product: ProductId,
        record: &TptBaseRecord,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let standards = write_standards(&record.standards);
        let thumbnails: Vec<Vec<u8>> = record
            .thumbnails
            .iter()
            .map(|handle| decode_hex(handle.as_str()))
            .collect::<Result<Vec<_>, StorageError>>()?;
        let video = record
            .video_preview
            .as_ref()
            .map(|handle| decode_hex(handle.as_str()))
            .transpose()?;

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "INSERT INTO product_tpt_base \
             (org_id, product_id, thumbnail_mode, thumbnail_hashes, video_preview_hash, \
              additional_licence_minor_units, bundle_discount_minor_units, tax_code_id, \
              subject_area_slugs, tag_slugs, format_slugs, custom_categories, standards, \
              teaching_duration_id, pages_or_slides, answer_key_id, \
              copyright_declaration_id, status_user, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
                     $17, $18, $19) \
             ON CONFLICT (org_id, product_id) DO UPDATE SET \
                 thumbnail_mode                 = EXCLUDED.thumbnail_mode, \
                 thumbnail_hashes               = EXCLUDED.thumbnail_hashes, \
                 video_preview_hash             = EXCLUDED.video_preview_hash, \
                 additional_licence_minor_units = EXCLUDED.additional_licence_minor_units, \
                 bundle_discount_minor_units    = EXCLUDED.bundle_discount_minor_units, \
                 tax_code_id                    = EXCLUDED.tax_code_id, \
                 subject_area_slugs             = EXCLUDED.subject_area_slugs, \
                 tag_slugs                      = EXCLUDED.tag_slugs, \
                 format_slugs                   = EXCLUDED.format_slugs, \
                 custom_categories              = EXCLUDED.custom_categories, \
                 standards                      = EXCLUDED.standards, \
                 teaching_duration_id           = EXCLUDED.teaching_duration_id, \
                 pages_or_slides                = EXCLUDED.pages_or_slides, \
                 answer_key_id                  = EXCLUDED.answer_key_id, \
                 copyright_declaration_id       = EXCLUDED.copyright_declaration_id, \
                 status_user                    = EXCLUDED.status_user, \
                 updated_at                     = EXCLUDED.updated_at",
            uuid_to_db(org.0),
            uuid_to_db(product.0),
            i16::from(record.thumbnail_mode.wire_id()),
            &thumbnails,
            video.as_deref(),
            record.additional_licence_minor_units,
            record.bundle_discount_minor_units,
            record.tax_code.map(|code| i16::from(code.wire_id())),
            &slug_strings(&record.categories.subject_areas),
            &slug_strings(&record.categories.tags),
            &slug_strings(&record.categories.formats),
            &record.categories.custom_categories,
            standards,
            record
                .details
                .teaching_duration
                .map(|duration| i16::from(duration.wire_id())),
            record
                .details
                .pages_or_slides
                .map(i32::try_from)
                .transpose()
                .map_err(|_| StorageError::Inconsistent {
                    reason: "the page count is larger than the column holds".to_owned(),
                })?,
            record
                .details
                .answer_key
                .map(|key| i16::from(key.wire_id())),
            record
                .copyright
                .map(|declaration| i16::from(declaration.wire_id())),
            i16::from(record.status.wire_id()),
            timestamp_to_db(now)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// This product's TPT-base fields, or `None` where none were written.
    ///
    /// `None` is the honest answer for a product authored before this table
    /// existed or imported from a marketplace, and is distinct from a product
    /// whose seller answered nothing, which is a row of nulls.
    pub async fn get(
        &self,
        org: OrgId,
        product: ProductId,
    ) -> Result<Option<TptBaseRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT thumbnail_mode, thumbnail_hashes, video_preview_hash, \
                    additional_licence_minor_units, bundle_discount_minor_units, tax_code_id, \
                    subject_area_slugs, tag_slugs, format_slugs, custom_categories, standards, \
                    teaching_duration_id, pages_or_slides, answer_key_id, \
                    copyright_declaration_id, status_user \
               FROM product_tpt_base WHERE org_id = $1 AND product_id = $2",
            uuid_to_db(org.0),
            uuid_to_db(product.0),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;

        let Some(row) = row else {
            return Ok(None);
        };
        let standards = read_standards(&row.standards)?;
        Ok(Some(TptBaseRecord {
            thumbnail_mode: decode(
                row.thumbnail_mode,
                ThumbnailMode::from_wire_id,
                "thumbnail mode",
            )?,
            thumbnails: row
                .thumbnail_hashes
                .iter()
                .map(|bytes| encode_hex(bytes))
                .collect::<Result<Vec<_>, StorageError>>()?,
            video_preview: row
                .video_preview_hash
                .as_deref()
                .map(encode_hex)
                .transpose()?,
            additional_licence_minor_units: row.additional_licence_minor_units,
            bundle_discount_minor_units: row.bundle_discount_minor_units,
            tax_code: decode_optional(row.tax_code_id, TaxCode::from_wire_id, "tax code")?,
            categories: CategoryGroup {
                // The grades live on `product.grades` as the verbatim
                // declaration every other reader already uses, so this row
                // does not hold a second copy of them.
                grades: vec![],
                subject_areas: slugs(&row.subject_area_slugs)?,
                tags: slugs(&row.tag_slugs)?,
                formats: slugs(&row.format_slugs)?,
                custom_categories: row.custom_categories,
            },
            standards,
            details: DetailGroup {
                teaching_duration: decode_optional(
                    row.teaching_duration_id,
                    TeachingDuration::from_wire_id,
                    "teaching duration",
                )?,
                pages_or_slides: row.pages_or_slides.map(u32::try_from).transpose().map_err(
                    |_| StorageError::CorruptRow {
                        reason: "the page count is negative".to_owned(),
                    },
                )?,
                answer_key: decode_optional(
                    row.answer_key_id,
                    AnswerKey::from_wire_id,
                    "answer key",
                )?,
            },
            copyright: decode_optional(
                row.copyright_declaration_id,
                CopyrightDeclaration::from_wire_id,
                "copyright declaration",
            )?,
            status: decode(
                row.status_user,
                ListingStatus::from_wire_id,
                "listing status",
            )?,
        }))
    }
}

fn slug_strings(slugs: &[FacetSlug]) -> Vec<String> {
    slugs.iter().map(|slug| slug.as_str().to_owned()).collect()
}

fn slugs(stored: &[String]) -> Result<Vec<FacetSlug>, StorageError> {
    stored
        .iter()
        .map(|value| {
            FacetSlug::new(value).map_err(|_| StorageError::CorruptRow {
                reason: "a stored facet slug is blank".to_owned(),
            })
        })
        .collect()
}

/// A stored wire id back through the domain's own membership test.
///
/// The CHECK constraints bound these columns, so a value outside the set means
/// the constraint was dropped or the row predates it. Naming that as corruption
/// is the point: the alternative is a listbox value silently reading as a
/// different member, which is the mis-map this vocabulary already has a
/// history of.
fn decode<T>(stored: i16, member: impl Fn(u8) -> Option<T>, what: &str) -> Result<T, StorageError> {
    u8::try_from(stored)
        .ok()
        .and_then(member)
        .ok_or_else(|| StorageError::CorruptRow {
            reason: format!("{stored} is not a {what} this form offers"),
        })
}

fn decode_optional<T>(
    stored: Option<i16>,
    member: impl Fn(u8) -> Option<T>,
    what: &str,
) -> Result<Option<T>, StorageError> {
    stored.map(|held| decode(held, member, what)).transpose()
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, StorageError> {
    if hex.len() != 64 {
        return Err(StorageError::Inconsistent {
            reason: "a file handle is not a 64-character hex digest".to_owned(),
        });
    }
    (0..32)
        .map(|index| {
            hex.get(index * 2..index * 2 + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| StorageError::Inconsistent {
                    reason: "a file handle is not hexadecimal".to_owned(),
                })
        })
        .collect()
}

fn encode_hex(bytes: &[u8]) -> Result<UploadRef, StorageError> {
    use core::fmt::Write;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // infallible on String; the Result is the trait's, not the writer's
        let _unused: core::fmt::Result = write!(hex, "{byte:02x}");
    }
    UploadRef::new(&hex).map_err(|_| StorageError::CorruptRow {
        reason: format!("a stored file handle is {} bytes, expected 32", bytes.len()),
    })
}
