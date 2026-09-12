//! The device-asserted content sketches, and the reads the matcher blocks on.
//!
//! Every column here is a claim a machine we do not operate made over bytes we
//! never held, in the shape and the voice `0052_product_file_source.sql` states
//! for `observed_hash`. This crate stores and retrieves them; it scores
//! nothing. The layers, the weights and the verdicts are the API's, because
//! they are a judgement about a seller's catalogue rather than a fact about a
//! row.
//!
//! The reads are blocking reads, in the entity-resolution sense: a cheap query
//! that narrows the org's catalogue to the pairs worth scoring. Three keys,
//! each with its own index: the four SimHash bands (Manku's technique, one
//! index per band), the payload digests, and the normalised title under
//! `pg_trgm`. A candidate reached by any one of them is scored by every layer.

use sqlx::PgPool;
use tam_types::{ContentHash, Marketplace, OrgId, PriceIntent, ProductId, Timestamp};

use crate::codec::{
    hash_from_db, hash_to_db, inventory_from_db, price_from_db, timestamp_to_db, uuid_from_db,
    uuid_to_db,
};
use crate::{pin_org, StorageError};

/// The text half of a sketch, which is one measurement: present together or
/// absent together, because a device that extracted nothing has not measured
/// zero shingles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSketchColumns {
    pub simhash: i64,
    /// 128 little-endian u32, fixed width, as the column's own CHECK states.
    pub minhash: Vec<u8>,
    pub shingle_count: i32,
    pub extracted_chars: i32,
    pub bands: [i16; 4],
}

/// One sketch to store.
#[derive(Debug, Clone)]
pub struct FingerprintWrite<'a> {
    pub product: ProductId,
    pub version: i16,
    pub text: Option<TextSketchColumns>,
    pub page_count: Option<i32>,
    pub cover_phash: Option<i64>,
    pub title_norm: &'a str,
    /// Which machine reported this, where one did. A spreadsheet row's sketch
    /// is computed from bytes the seller uploaded to us and names no device.
    pub observed_by_device: Option<&'a str>,
    pub observed_at: Timestamp,
}

/// One candidate the blocking reached, with everything the layers need from
/// the sketch and the product row.
#[derive(Debug, Clone)]
pub struct CandidateSketch {
    pub product: ProductId,
    pub title: String,
    pub price: PriceIntent,
    pub text: Option<TextSketchColumns>,
    pub page_count: Option<i32>,
    pub cover_phash: Option<i64>,
    pub title_norm: String,
    /// Postgres's own trigram similarity between this title and the one the
    /// blocking asked about, so the L4 trigram arm is measured where the index
    /// is rather than re-implemented beside it.
    pub title_similarity: f32,
}

/// One payload file's device-asserted digest.
///
/// `observed_hash` where the bytes were the seller's and stayed there;
/// `hash` where we hold them. One row per live payload file, because L1 is an
/// equality over entries and L1b a Jaccard over the set of them.
#[derive(Debug, Clone)]
pub struct PayloadDigest {
    pub product: ProductId,
    pub digest: ContentHash,
    pub byte_len: u64,
    pub name: Option<String>,
}

/// What a product's own metadata says, for the corroboration layer.
#[derive(Debug, Clone)]
pub struct ProductMeta {
    pub product: ProductId,
    pub subjects: Vec<uuid::Uuid>,
    pub grade_low: Option<i16>,
    pub grade_high: Option<i16>,
    /// Every marketplace this product is mapped onto. Empty for a product no
    /// mapping names, which the cross-marketplace rule treats as comparable
    /// with anything: a catalogue-only resource has no marketplace to be on
    /// the same side of.
    pub marketplaces: Vec<Marketplace>,
}

pub struct FingerprintRepo {
    pool: PgPool,
}

impl FingerprintRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Stores one product's sketch, replacing the one it held at this version.
    ///
    /// Replacing rather than appending, because the primary key is the version:
    /// a re-import of the same resource is a fresh measurement of the same
    /// bytes under the same format, and two rows would give the matcher two
    /// answers about one product.
    pub async fn put(
        &self,
        org: OrgId,
        write: &FingerprintWrite<'_>,
        recorded_at: Timestamp,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        put_fingerprint(&mut tx, org, write, recorded_at).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The candidates any one blocking key reaches.
    ///
    /// The bands and the title are one query rather than two, because a
    /// candidate reached by both must be scored once: two queries unioned in
    /// Rust would either duplicate it or need a de-duplication pass that the
    /// database does for nothing.
    #[expect(
        clippy::too_many_arguments,
        reason = "each parameter is one blocking key or its version; a struct would name the \
                  argument list and nothing else, and every caller passes all of them"
    )]
    pub async fn candidates(
        &self,
        org: OrgId,
        version: i16,
        bands: Option<[i16; 4]>,
        title_norm: &str,
        also: &[ProductId],
    ) -> Result<Vec<CandidateSketch>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let found = candidates_in(&mut tx, org, version, bands, title_norm, also).await?;
        tx.commit().await?;
        Ok(found)
    }

    /// Which products carry any of these payload digests, and how many
    /// products each digest is on.
    ///
    /// The count is the frequency weighting the research insists on: a
    /// seller's `Terms of Use.pdf` is byte-identical across their whole
    /// catalogue, and an unweighted digest equality would merge the lot.
    pub async fn products_by_digest(
        &self,
        org: OrgId,
        digests: &[ContentHash],
    ) -> Result<Vec<PayloadDigest>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let found = products_by_digest_in(&mut tx, org, digests).await?;
        tx.commit().await?;
        Ok(found)
    }

    /// Every live payload digest these products carry, so L1b's Jaccard is
    /// over sets rather than over one equality.
    pub async fn digests_for(
        &self,
        org: OrgId,
        products: &[ProductId],
    ) -> Result<Vec<PayloadDigest>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let found = digests_for_in(&mut tx, org, products).await?;
        tx.commit().await?;
        Ok(found)
    }

    /// How many distinct live products each digest is on, org-wide.
    pub async fn digest_frequency(
        &self,
        org: OrgId,
        digests: &[ContentHash],
    ) -> Result<Vec<(ContentHash, u32)>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let found = digest_frequency_in(&mut tx, org, digests).await?;
        tx.commit().await?;
        Ok(found)
    }

    /// How many products share one normalised title, which is L4's own
    /// frequency weighting: a seller whose whole range is called
    /// "reading comprehension" has told us nothing by agreeing on it.
    pub async fn title_frequency(
        &self,
        org: OrgId,
        version: i16,
        title_norm: &str,
    ) -> Result<u32, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let held = title_frequency_in(&mut tx, org, version, title_norm).await?;
        tx.commit().await?;
        Ok(held)
    }

    /// The subjects, the grade interval and the marketplaces of each product,
    /// for the corroboration layer and the cross-marketplace rule.
    pub async fn metadata_for(
        &self,
        org: OrgId,
        products: &[ProductId],
    ) -> Result<Vec<ProductMeta>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let found = metadata_for_in(&mut tx, org, products).await?;
        tx.commit().await?;
        Ok(found)
    }
}

fn find_mut(rows: &mut [ProductMeta], product: uuid::Uuid) -> Option<&mut ProductMeta> {
    let product = ProductId(uuid_from_db(product));
    rows.iter_mut().find(|meta| meta.product == product)
}

// The matcher's own reads, each reachable inside a transaction the caller
// owns. That is what lets the commit re-ask the matcher under the
// organisation's catalogue lock, against what is committed at that instant
// rather than against what the catalogue held when the page landed. Every
// repository method above is one of these with a transaction opened around
// it, so there is one implementation of each read.

/// The candidates any one blocking key reaches, inside a transaction.
#[expect(
    clippy::too_many_arguments,
    reason = "each parameter is one blocking key or its version; a struct would name the argument list and nothing else, and every caller passes all of them"
)]
pub async fn candidates_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    version: i16,
    bands: Option<[i16; 4]>,
    title_norm: &str,
    also: &[ProductId],
) -> Result<Vec<CandidateSketch>, StorageError> {
    let also: Vec<uuid::Uuid> = also.iter().map(|id| uuid_to_db(id.0)).collect();
    let bands: Option<Vec<i16>> = bands.map(|bands| bands.to_vec());
    let rows = sqlx::query!(
        "SELECT f.product_id, p.title, p.price_kind, p.price_minor_units, \
                p.price_currency, f.text_simhash, f.text_minhash, f.shingle_count, \
                f.extracted_chars, f.simhash_band_0, f.simhash_band_1, \
                f.simhash_band_2, f.simhash_band_3, f.page_count, f.cover_phash, \
                f.title_norm, similarity(f.title_norm, $4) AS title_similarity \
           FROM product_fingerprint f \
           JOIN product p ON p.org_id = f.org_id AND p.id = f.product_id \
          WHERE f.org_id = $1 AND f.fingerprint_version = $2 AND p.deleted_at IS NULL \
            AND ( \
                 ($3::smallint[] IS NOT NULL \
                    AND (f.simhash_band_0 = ($3::smallint[])[1] \
                      OR f.simhash_band_1 = ($3::smallint[])[2] \
                      OR f.simhash_band_2 = ($3::smallint[])[3] \
                      OR f.simhash_band_3 = ($3::smallint[])[4])) \
              OR f.title_norm = $4 \
              OR similarity(f.title_norm, $4) >= 0.5 \
              OR f.product_id = ANY($5) \
            )",
        uuid_to_db(org.0),
        version,
        bands.as_deref(),
        title_norm,
        &also,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            let text = match (
                row.text_simhash,
                row.text_minhash,
                row.shingle_count,
                row.extracted_chars,
                row.simhash_band_0,
                row.simhash_band_1,
                row.simhash_band_2,
                row.simhash_band_3,
            ) {
                (
                    Some(simhash),
                    Some(minhash),
                    Some(shingle_count),
                    Some(extracted_chars),
                    Some(band_0),
                    Some(band_1),
                    Some(band_2),
                    Some(band_3),
                ) => Some(TextSketchColumns {
                    simhash,
                    minhash,
                    shingle_count,
                    extracted_chars,
                    bands: [band_0, band_1, band_2, band_3],
                }),
                _ => None,
            };
            Ok(CandidateSketch {
                product: ProductId(uuid_from_db(row.product_id)),
                title: row.title,
                price: price_from_db(&row.price_kind, row.price_minor_units, row.price_currency)?,
                text,
                page_count: row.page_count,
                cover_phash: row.cover_phash,
                title_norm: row.title_norm,
                title_similarity: row.title_similarity.unwrap_or(0.0),
            })
        })
        .collect()
}

/// Which products carry any of these payload digests, inside a transaction.
pub async fn products_by_digest_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    digests: &[ContentHash],
) -> Result<Vec<PayloadDigest>, StorageError> {
    if digests.is_empty() {
        return Ok(Vec::new());
    }
    let digests: Vec<Vec<u8>> = digests.iter().copied().map(hash_to_db).collect();
    let rows = sqlx::query!(
        "SELECT f.product_id, \
                COALESCE(f.observed_hash, f.hash) AS digest, \
                COALESCE(f.observed_byte_len, b.byte_len) AS byte_len, \
                COALESCE(f.payload_file_name, f.name) AS file_name \
           FROM product_file f \
           JOIN product p ON p.org_id = f.org_id AND p.id = f.product_id \
           LEFT JOIN blob b ON b.org_id = f.org_id AND b.hash = f.hash \
          WHERE f.org_id = $1 AND f.role = 'payload' AND f.deleted_at IS NULL \
            AND p.deleted_at IS NULL \
            AND COALESCE(f.observed_hash, f.hash) = ANY($2)",
        uuid_to_db(org.0),
        &digests,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(PayloadDigest {
                product: ProductId(uuid_from_db(row.product_id)),
                digest: hash_from_db(row.digest.as_deref().unwrap_or_default())?,
                byte_len: u64::try_from(row.byte_len.unwrap_or(0)).unwrap_or(0),
                name: row.file_name,
            })
        })
        .collect()
}

/// Every live payload digest these products carry, inside a transaction.
pub async fn digests_for_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    products: &[ProductId],
) -> Result<Vec<PayloadDigest>, StorageError> {
    if products.is_empty() {
        return Ok(Vec::new());
    }
    let products: Vec<uuid::Uuid> = products.iter().map(|id| uuid_to_db(id.0)).collect();
    let rows = sqlx::query!(
        "SELECT f.product_id, \
                COALESCE(f.observed_hash, f.hash) AS digest, \
                COALESCE(f.observed_byte_len, b.byte_len) AS byte_len, \
                COALESCE(f.payload_file_name, f.name) AS file_name \
           FROM product_file f \
           LEFT JOIN blob b ON b.org_id = f.org_id AND b.hash = f.hash \
          WHERE f.org_id = $1 AND f.role = 'payload' AND f.deleted_at IS NULL \
            AND f.product_id = ANY($2)",
        uuid_to_db(org.0),
        &products,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(PayloadDigest {
                product: ProductId(uuid_from_db(row.product_id)),
                digest: hash_from_db(row.digest.as_deref().unwrap_or_default())?,
                byte_len: u64::try_from(row.byte_len.unwrap_or(0)).unwrap_or(0),
                name: row.file_name,
            })
        })
        .collect()
}

/// How many distinct live products each digest is on, inside a transaction.
pub async fn digest_frequency_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    digests: &[ContentHash],
) -> Result<Vec<(ContentHash, u32)>, StorageError> {
    if digests.is_empty() {
        return Ok(Vec::new());
    }
    let digests: Vec<Vec<u8>> = digests.iter().copied().map(hash_to_db).collect();
    let rows = sqlx::query!(
        "SELECT COALESCE(f.observed_hash, f.hash) AS digest, \
                count(DISTINCT f.product_id)::bigint AS held \
           FROM product_file f \
           JOIN product p ON p.org_id = f.org_id AND p.id = f.product_id \
          WHERE f.org_id = $1 AND f.role = 'payload' AND f.deleted_at IS NULL \
            AND p.deleted_at IS NULL \
            AND COALESCE(f.observed_hash, f.hash) = ANY($2) \
          GROUP BY 1",
        uuid_to_db(org.0),
        &digests,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok((
                hash_from_db(row.digest.as_deref().unwrap_or_default())?,
                u32::try_from(row.held.unwrap_or(0)).unwrap_or(u32::MAX),
            ))
        })
        .collect()
}

/// How many products share one normalised title, inside a transaction.
pub async fn title_frequency_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    version: i16,
    title_norm: &str,
) -> Result<u32, StorageError> {
    let held: i64 = sqlx::query_scalar!(
        "SELECT count(*)::bigint FROM product_fingerprint f \
           JOIN product p ON p.org_id = f.org_id AND p.id = f.product_id \
          WHERE f.org_id = $1 AND f.fingerprint_version = $2 \
            AND p.deleted_at IS NULL AND f.title_norm = $3",
        uuid_to_db(org.0),
        version,
        title_norm,
    )
    .fetch_one(&mut **tx)
    .await?
    .unwrap_or(0);
    Ok(u32::try_from(held).unwrap_or(u32::MAX))
}

/// The subjects, the grade interval and the marketplaces of each product,
/// inside a transaction.
pub async fn metadata_for_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    products: &[ProductId],
) -> Result<Vec<ProductMeta>, StorageError> {
    if products.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<uuid::Uuid> = products.iter().map(|id| uuid_to_db(id.0)).collect();
    let terms = sqlx::query!(
        "SELECT product_id, term_id FROM product_term \
          WHERE org_id = $1 AND product_id = ANY($2)",
        uuid_to_db(org.0),
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    let grades = sqlx::query!(
        "SELECT product_id, derived_low_years, derived_high_years FROM grade_declaration \
          WHERE org_id = $1 AND product_id = ANY($2)",
        uuid_to_db(org.0),
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    let mappings = sqlx::query!(
        "SELECT product_id, inventory FROM mapping \
          WHERE org_id = $1 AND product_id = ANY($2)",
        uuid_to_db(org.0),
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;

    let mut out: Vec<ProductMeta> = products
        .iter()
        .map(|product| ProductMeta {
            product: *product,
            subjects: Vec::new(),
            grade_low: None,
            grade_high: None,
            marketplaces: Vec::new(),
        })
        .collect();
    for row in terms {
        if let Some(meta) = find_mut(&mut out, row.product_id) {
            meta.subjects.push(row.term_id);
        }
    }
    for row in grades {
        if let Some(meta) = find_mut(&mut out, row.product_id) {
            meta.grade_low = row.derived_low_years;
            meta.grade_high = row.derived_high_years;
        }
    }
    for row in mappings {
        let marketplace = inventory_from_db(&row.inventory)?.marketplace();
        if let Some(meta) = find_mut(&mut out, row.product_id) {
            if !meta.marketplaces.contains(&marketplace) {
                meta.marketplaces.push(marketplace);
            }
        }
    }
    Ok(out)
}

/// Stores one product's sketch inside a transaction the caller owns.
///
/// The one implementation of the sketch write. The import's commit writes it
/// beside the product it describes, because a sketch that survived a
/// rolled-back product is a candidate the matcher would offer for a resource
/// nobody has. [`FingerprintRepo::put`] is this call with a transaction
/// opened around it.
pub async fn put_fingerprint(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    write: &FingerprintWrite<'_>,
    recorded_at: Timestamp,
) -> Result<(), StorageError> {
    let text = write.text.as_ref();
    sqlx::query!(
        "INSERT INTO product_fingerprint \
           (org_id, product_id, fingerprint_version, text_simhash, text_minhash, \
            shingle_count, extracted_chars, simhash_band_0, simhash_band_1, \
            simhash_band_2, simhash_band_3, page_count, cover_phash, title_norm, \
            observed_by_device, observed_at, recorded_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17) \
         ON CONFLICT (org_id, product_id, fingerprint_version) DO UPDATE SET \
           text_simhash = EXCLUDED.text_simhash, \
           text_minhash = EXCLUDED.text_minhash, \
           shingle_count = EXCLUDED.shingle_count, \
           extracted_chars = EXCLUDED.extracted_chars, \
           simhash_band_0 = EXCLUDED.simhash_band_0, \
           simhash_band_1 = EXCLUDED.simhash_band_1, \
           simhash_band_2 = EXCLUDED.simhash_band_2, \
           simhash_band_3 = EXCLUDED.simhash_band_3, \
           page_count = EXCLUDED.page_count, \
           cover_phash = EXCLUDED.cover_phash, \
           title_norm = EXCLUDED.title_norm, \
           observed_by_device = EXCLUDED.observed_by_device, \
           observed_at = EXCLUDED.observed_at, \
           recorded_at = EXCLUDED.recorded_at",
        uuid_to_db(org.0),
        uuid_to_db(write.product.0),
        write.version,
        text.map(|text| text.simhash),
        text.map(|text| text.minhash.as_slice()),
        text.map(|text| text.shingle_count),
        text.map(|text| text.extracted_chars),
        text.map(|text| text.bands[0]),
        text.map(|text| text.bands[1]),
        text.map(|text| text.bands[2]),
        text.map(|text| text.bands[3]),
        write.page_count,
        write.cover_phash,
        write.title_norm,
        write.observed_by_device,
        timestamp_to_db(write.observed_at)?,
        timestamp_to_db(recorded_at)?,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
