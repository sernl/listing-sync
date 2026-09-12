//! The catalogue aggregate: a product with its files, blobs, taxonomy
//! assignments and grade declaration, written and read as one unit inside one
//! pinned transaction. The deferred `assert_product_has_payload` trigger
//! re-states `PayloadSet`'s non-emptiness at commit, so a partial write
//! cannot leave a payload-less product behind.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::collections::HashMap;
use tam_domain::{
    AgeInterval, CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration,
    TermKind, VocabularyId, VocabularyPath,
};
use tam_marketplace::RemoteListingId;
use tam_types::{
    CanonicalTermId, ContentHash, FileId, FileRole, ImportedTerm, InventoryId, ListingCopy, OrgId,
    PayloadSet, PriceIntent, ProductFile, ProductId, Timestamp, Title,
};

use crate::codec::{
    copy_format_from_db, copy_format_to_db, file_kind_from_db, file_kind_to_db, file_role_from_db,
    file_role_to_db, hash_from_db, hash_hex, hash_to_db, inventory_from_db, inventory_to_db,
    price_from_db, scan_from_db, term_kind_from_db, term_kind_to_db, timestamp_from_db,
    timestamp_to_db, uuid_from_db, uuid_to_db, PriceColumns, ScanColumns,
};
use crate::mapping::remote_id_from_db;
use crate::{pin_org, StorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductRecord {
    pub product: CanonicalProduct,
    /// What the seller called each file they named, keyed by file.
    ///
    /// A map rather than a field on `ProductFile`, and absent for every file
    /// stored before the name column existed: the bytes were sealed under a
    /// content hash and the filename was never recorded anywhere, so there is
    /// nothing to backfill from and a reader has to render the absence rather
    /// than invent a name for it.
    pub file_names: HashMap<FileId, String>,
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

/// What one resource brings to a listing that does not exist yet.
///
/// Read by [`ProductRepo::creation_facts`] and decided on by the caller. Every
/// member is something the catalogue holds rather than something a target
/// requires: which marketplace declares which field is the registry's answer,
/// and a copy of it down here would be a second one to keep in step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductCreationFacts {
    pub product: ProductId,
    /// How many undeleted payload files it carries. Zero is a resource kept
    /// on Teachouse alone (D32), which no marketplace listing can be made of.
    pub payload_files: usize,
    /// The marketplaces holding those bytes, where any of them are the
    /// seller's own file on a shop rather than bytes we sealed. Empty for a
    /// wholly blob-backed payload, and never longer than the payload set.
    pub payload_sources: Vec<tam_types::Marketplace>,
    /// Whether the product carries a rights declaration of its own.
    pub rights_declared: bool,
    /// The axes this tenant has already settled for the inventory the facts
    /// were read against, which is the other way a required field is met.
    pub settled_axes: Vec<TermKind>,
}

/// One resource as the catalogue export writes it: the product's own columns,
/// the labels it carries and one entry per marketplace listing.
///
/// A shape of its own rather than a composition of [`ProductSummary`],
/// `LabelRepo::for_product` and `MappingRepo::list_for_product`, because that
/// composition cost two transactions and roughly eighteen statements for every
/// resource in the catalogue: `list_for_product` hydrates each mapping's field
/// mismatches and binding candidates, and an export reads neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedResource {
    pub id: ProductId,
    pub title: Title,
    pub price: PriceIntent,
    /// The seller's own labels, ordered as the console lists them.
    pub labels: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub listings: Vec<ExportedListing>,
}

/// One marketplace listing of one resource, in the terms an export renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedListing {
    pub inventory: InventoryId,
    /// The stored spellings, which are what `MappingHead` publishes and what
    /// the console's listing vocabulary is read from.
    pub binding_state: String,
    pub lifecycle_state: String,
    /// Decoded only while the binding is `bound`, as `MappingHead` does it: a
    /// severed mapping still carries remote-id columns, and the console reads
    /// severed as not listed.
    pub remote: Option<RemoteListingId>,
    /// The price the seller set for this inventory by hand.
    ///
    /// Absent where the rule converts the catalogue price instead, because a
    /// converted rule records a rate rather than an amount and nothing in this
    /// workspace derives the amount from it.
    pub listed_price: Option<PriceIntent>,
}

/// One edit to a product's canonical fields. Every field is optional and an
/// absent one is left as stored, so a partial form submission cannot erase
/// what it did not render.
///
/// Files are deliberately absent: bytes enter through the upload path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProductEdit {
    pub title: Option<Title>,
    pub body: Option<ListingCopy>,
    pub price: Option<PriceIntent>,
    pub subjects: Option<Vec<CanonicalTermId>>,
    pub grades: Option<GradeDeclaration>,
    pub rights: Option<RightsDeclaration>,
}

/// Files are deliberately absent from [`ProductEdit`] and reached by the three
/// methods below instead, because a file is not a field of the copy that
/// describes it: an edit merges what it carries and leaves the rest, while
/// removing a file has to be expressible and a merge cannot express it.
///
/// Why one mutation did not happen. Every variant is the seller's to fix or
/// the client's to have avoided, which is why they sit inside the `Ok` arm
/// rather than beside [`StorageError`]: the outer arm is a fault of ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRefusal {
    /// This tenant holds no live product of that identifier.
    NoProduct,
    /// The product holds no live file of that identifier.
    NoFile,
    /// The removal would leave the product with no payload file, which
    /// `assert_product_has_payload` forbids and no listing could survive.
    LastPayload,
    /// The product already carries a live cover, and
    /// `product_file_one_cover` admits one. Replace it rather than add a
    /// second.
    CoverExists,
}

/// The file one mutation addresses. A file is named within its product rather
/// than on its own, which is what makes another tenant's identifier — or this
/// tenant's, on the wrong product — a plain not-found rather than a row
/// reachable by guessing a uuid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileTarget {
    pub product: ProductId,
    pub file: FileId,
}

/// The parts of a file a replacement supplies.
///
/// The role is not among them on purpose: a replacement keeps the role of the
/// row it replaces, so a caller able to state a role would be able to state
/// the wrong one, and the only way for it to know the right one would be a
/// read it would then have to trust across the gap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReplacement {
    pub id: FileId,
    pub kind: tam_types::FileKind,
    pub bytes: tam_types::FileBytes,
    /// What the seller called the file they chose, where they chose one. A
    /// cover carries none: nobody chooses a generated picture.
    pub name: Option<String>,
}

/// One replacement, and the cover to redraw with it.
///
/// The cover rides along rather than travelling as a second call because a
/// cover drawn from a file the product no longer holds is exactly the state
/// the two writes have to skip over, and two transactions cannot.
///
/// Offering the cover is not the same as writing it: [`ProductRepo::
/// replace_file`] writes it only where the file being replaced is the one a
/// cover would have been drawn from, so a caller may always offer it and
/// never has to decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSwap {
    pub file: FileReplacement,
    pub cover: Option<FileReplacement>,
}

/// What became of a product's thumbnail when one of its files was removed.
///
/// Three states rather than an `Option`, because a removal can leave the
/// thumbnail alone, replace it, or take it away, and the third is not the
/// absence of the second: a resource that loses its only file has nothing left
/// to draw a thumbnail from, so the old one is retired rather than redrawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThumbnailChange {
    Untouched,
    /// Boxed because a `ProductFile` is two hundred-odd bytes and the other
    /// two arms carry none: every caller would otherwise pay for the largest.
    Redrawn(Box<ProductFile>),
    Retired,
}

/// What one replace wrote. `cover` is present only where the cover was
/// redrawn, so a caller states that it was rather than assuming it from
/// having offered one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacedFiles {
    pub file: ProductFile,
    pub cover: Option<ProductFile>,
}

pub struct ProductRepo {
    pool: PgPool,
}

impl ProductRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Writes a product whose files nobody named, which is every path that
    /// does not come from a seller choosing a file: the operator import, the
    /// device import, and every fixture.
    pub async fn insert(
        &self,
        org: OrgId,
        product: &CanonicalProduct,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        self.insert_named(org, product, &HashMap::new(), at).await
    }

    /// The same write, carrying what the seller called each file.
    ///
    /// A separate entry point rather than a fifth parameter on `insert`,
    /// because every existing caller of that has no names to give and
    /// threading `&HashMap::new()` through each of them would say nothing.
    /// A name for a file the product does not carry is ignored rather than
    /// refused: the map is a lookup, not an assertion about the file set.
    pub async fn insert_named(
        &self,
        org: OrgId,
        product: &CanonicalProduct,
        names: &HashMap<FileId, String>,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        insert_product(&mut tx, org, product, names, at).await?;
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
            // A LEFT JOIN, because a marketplace-sourced file has no blob row
            // and an inner one would not merely omit its length — it would
            // drop the file from the product entirely, so a device-imported
            // product would read back with its payload missing everywhere the
            // catalogue is read from, the console and the manifest builder
            // included.
            "SELECT f.id, f.role, f.kind, f.hash, b.byte_len AS \"byte_len?\", \
             f.scan_state, f.scan_signature, f.scanned_at, f.scan_failure_code, \
             f.source_marketplace, f.source_connection, f.source_resource, f.source_entry, \
             f.observed_hash, f.observed_byte_len, f.observed_by_device, f.observed_at, \
             f.asserted_scan_state, f.asserted_scan_signature, \
             f.asserted_scanned_at, f.asserted_scan_failure_code, \
             f.payload_file_name, f.payload_content_type, f.name \
             FROM product_file f \
             LEFT JOIN blob b ON b.org_id = f.org_id AND b.hash = f.hash \
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
        let ProductFiles {
            payload,
            cover,
            previews,
            names,
        } = partition_files(files)?;
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
            file_names: names,
            created_at: timestamp_from_db(row.created_at),
            updated_at: timestamp_from_db(row.updated_at),
        }))
    }

    /// Applies one edit to a product's canonical fields, in one transaction.
    ///
    /// Files are absent from [`ProductEdit`] on purpose: bytes enter through
    /// the upload path, and swapping a payload is a different operation from
    /// editing the copy that describes it. Every field is optional and an
    /// absent one is left as stored, so a client that renders four fields
    /// cannot erase the two it does not.
    ///
    /// The merge happens inside the statement rather than in a caller that
    /// read first: a read-modify-write across two round trips loses the
    /// concurrent edit that landed between them.
    ///
    /// `false` means no live product of that identifier exists for the
    /// tenant, which is the not-found answer rather than a fault.
    pub async fn update(
        &self,
        org: OrgId,
        id: ProductId,
        edit: &ProductEdit,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let touched = update_product(&mut tx, org, id, edit, at).await?;
        tx.commit().await?;
        Ok(touched)
    }

    /// Which of these content hashes this tenant actually holds bytes for, and
    /// how long each stored blob is.
    ///
    /// The create names handles a client holds, and `insert_file` upserts a
    /// `blob` row rather than requiring one — the not-yet-encrypted sentinel
    /// path M1f left behind — so without this a handle naming bytes nobody
    /// uploaded would mint a phantom row pointing at an object that does not
    /// exist, and count against the tenant's storage. Asked here rather than
    /// on `BlobRepo` because the caller is the create, which needs no object
    /// store to answer it.
    ///
    /// The length travels with the hash because a caller that reads one of
    /// these blobs back has to bound the read before it makes it, and
    /// `blob.byte_len` is the only length in the system that no client
    /// asserted: a `FileHandle` carries one the request body sent, and a bare
    /// thumbnail digest carries none at all.
    pub async fn stored_hashes(
        &self,
        org: OrgId,
        hashes: &[tam_types::ContentHash],
    ) -> Result<Vec<(tam_types::ContentHash, i64)>, StorageError> {
        let wanted: Vec<Vec<u8>> = hashes.iter().copied().map(hash_to_db).collect();
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT hash, byte_len FROM blob WHERE org_id = $1 AND hash = ANY($2)",
            uuid_to_db(org.0),
            &wanted,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.iter()
            .map(|row| Ok((hash_from_db(&row.hash)?, row.byte_len)))
            .collect()
    }

    /// Adds one file to a product that already exists.
    ///
    /// The new row takes `MAX(position) + 1` rather than the number of live
    /// rows, because `product_file_position` is not a partial index: a
    /// soft-deleted row keeps its position, and reusing it collides.
    pub async fn add_file(
        &self,
        org: OrgId,
        product: ProductId,
        added: (&ProductFile, Option<&str>),
        at: Timestamp,
    ) -> Result<Result<FileId, FileRefusal>, StorageError> {
        let (file, name) = added;
        let org_db = uuid_to_db(org.0);
        let product_db = uuid_to_db(product.0);
        let at_db = timestamp_to_db(at)?;

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        if !lock_product(&mut tx, org_db, product_db).await? {
            tx.rollback().await?;
            return Ok(Err(FileRefusal::NoProduct));
        }
        if file.role == FileRole::Cover && live_cover(&mut tx, org_db, product_db).await?.is_some()
        {
            tx.rollback().await?;
            return Ok(Err(FileRefusal::CoverExists));
        }
        let position = next_position(&mut tx, org_db, product_db).await?;
        let write = FileWrite {
            org: org_db,
            product: product_db,
            at: at_db,
        };
        insert_file(
            &mut tx,
            &write,
            NewFile {
                position,
                slot: file.role,
                file,
                name: name.filter(|_| matches!(file.bytes, tam_types::FileBytes::Held { .. })),
            },
        )
        .await?;
        touch(&mut tx, org_db, product_db, at_db).await?;
        tx.commit().await?;
        Ok(Ok(file.id))
    }

    /// Swaps one file's bytes for another's, keeping the role it occupies.
    ///
    /// The old row is retired before the new one is written, which is what
    /// lets a cover be replaced at all: `product_file_one_cover` is an
    /// immediate partial unique index, so the two rows cannot both be live
    /// even for the length of a statement.
    pub async fn replace_file(
        &self,
        org: OrgId,
        target: FileTarget,
        swap: &FileSwap,
        at: Timestamp,
    ) -> Result<Result<ReplacedFiles, FileRefusal>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let product_db = uuid_to_db(target.product.0);
        let file_db = uuid_to_db(target.file.0);
        let at_db = timestamp_to_db(at)?;

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        if !lock_product(&mut tx, org_db, product_db).await? {
            tx.rollback().await?;
            return Ok(Err(FileRefusal::NoProduct));
        }
        let Some(role) = live_role(&mut tx, org_db, product_db, file_db).await? else {
            tx.rollback().await?;
            return Ok(Err(FileRefusal::NoFile));
        };
        // The cover is drawn from the first payload file, so replacing that
        // file and not the cover leaves a picture of a file the product no
        // longer holds. Decided here rather than by the caller: the caller
        // cannot see the positions, and a caller that guessed would redraw
        // the cover from the second payload file.
        let redraw = match (&swap.cover, role) {
            (Some(cover), FileRole::Payload)
                if first_payload(&mut tx, org_db, product_db).await? == Some(file_db) =>
            {
                // Only where one already lives. A product with no cover is
                // not given one by an edit that was asked to replace a
                // different file.
                live_cover(&mut tx, org_db, product_db)
                    .await?
                    .map(|existing| (existing, cover))
            }
            _ => None,
        };

        let write = FileWrite {
            org: org_db,
            product: product_db,
            at: at_db,
        };
        retire(&mut tx, org_db, product_db, file_db, at_db).await?;
        let written = ProductFile {
            id: swap.file.id,
            role,
            kind: swap.file.kind,
            bytes: swap.file.bytes.clone(),
        };
        let position = next_position(&mut tx, org_db, product_db).await?;
        insert_file(
            &mut tx,
            &write,
            NewFile {
                position,
                slot: role,
                file: &written,
                name: swap.file.name.as_deref(),
            },
        )
        .await?;

        let mut cover = None;
        if let Some((existing, replacement)) = redraw {
            // Retired before the new one is written, because
            // `product_file_one_cover` is immediate: the two cannot both be
            // live even for the length of a statement.
            retire(&mut tx, org_db, product_db, existing, at_db).await?;
            let drawn = ProductFile {
                id: replacement.id,
                role: FileRole::Cover,
                kind: replacement.kind,
                bytes: replacement.bytes.clone(),
            };
            let position = next_position(&mut tx, org_db, product_db).await?;
            insert_file(
                &mut tx,
                &write,
                NewFile {
                    position,
                    slot: FileRole::Cover,
                    file: &drawn,
                    name: None,
                },
            )
            .await?;
            cover = Some(drawn);
        }

        touch(&mut tx, org_db, product_db, at_db).await?;
        tx.commit().await?;
        Ok(Ok(ReplacedFiles {
            file: written,
            cover,
        }))
    }

    /// Retires one file, refusing the removal that would leave the product
    /// with no payload at all.
    ///
    /// The count is taken here rather than left to `product_file_payload_
    /// nonempty`, for two reasons. The trigger raises a `check_violation` a
    /// caller would have to read out of an error string to turn into a
    /// sentence, and it is deferred, so by the time it fires the transaction
    /// is already lost. It also does not hold on its own: two concurrent
    /// removals each see the other's row as live, so each passes its own
    /// trigger and the product ends with none. The `FOR UPDATE` on the
    /// product row is what closes that, by serialising file writes per
    /// product; the trigger stays as the backstop for every other path.
    pub async fn remove_file(
        &self,
        org: OrgId,
        target: FileTarget,
        redrawn: Option<&FileReplacement>,
        at: Timestamp,
    ) -> Result<Result<ThumbnailChange, FileRefusal>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let product_db = uuid_to_db(target.product.0);
        let file_db = uuid_to_db(target.file.0);
        let at_db = timestamp_to_db(at)?;

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        if !lock_product(&mut tx, org_db, product_db).await? {
            tx.rollback().await?;
            return Ok(Err(FileRefusal::NoProduct));
        }
        let Some(role) = live_role(&mut tx, org_db, product_db, file_db).await? else {
            tx.rollback().await?;
            return Ok(Err(FileRefusal::NoFile));
        };
        // The last payload file may go where no marketplace carries the
        // resource, which is what migration 0061 moved: the requirement is a
        // property of the mapping rather than of the product, so a draft kept
        // here alone is allowed to have no file yet. The count is taken inside
        // this transaction and under the same lock, so a mapping added while
        // the removal was in flight cannot leave a listed resource fileless.
        let last_payload =
            role == FileRole::Payload && live_payloads(&mut tx, org_db, product_db).await? <= 1;
        if last_payload && has_mapping(&mut tx, org_db, product_db).await? {
            tx.rollback().await?;
            return Ok(Err(FileRefusal::LastPayload));
        }
        // Removing the file the thumbnail was drawn from leaves it depicting a
        // file the product no longer holds — the same staleness
        // `replace_file` redraws away, through the door that was left open.
        // The caller renders the replacement from the payload file that
        // becomes the first one; the condition is re-decided here under the
        // lock, so a thumbnail drawn against a product that moved underneath
        // is discarded rather than written.
        // The thumbnail is drawn from the first payload file, so removing that
        // file leaves it depicting a file the product no longer holds. What to
        // do about it depends on what is left: another payload file to draw
        // from means a redraw, and nothing left means the thumbnail goes too,
        // because a picture of a file the resource does not have is worse than
        // no picture.
        let drawn_from_this = role == FileRole::Payload
            && first_payload(&mut tx, org_db, product_db).await? == Some(file_db);
        let standing = if drawn_from_this {
            live_cover(&mut tx, org_db, product_db).await?
        } else {
            None
        };

        retire(&mut tx, org_db, product_db, file_db, at_db).await?;
        let mut outcome = ThumbnailChange::Untouched;
        if let Some(existing) = standing {
            match (last_payload, redrawn) {
                (true, _) | (false, None) => {
                    retire(&mut tx, org_db, product_db, existing, at_db).await?;
                    outcome = ThumbnailChange::Retired;
                }
                (false, Some(replacement)) => {
                    retire(&mut tx, org_db, product_db, existing, at_db).await?;
                    let drawn = ProductFile {
                        id: replacement.id,
                        role: FileRole::Cover,
                        kind: replacement.kind,
                        bytes: replacement.bytes.clone(),
                    };
                    let position = next_position(&mut tx, org_db, product_db).await?;
                    insert_file(
                        &mut tx,
                        &FileWrite {
                            org: org_db,
                            product: product_db,
                            at: at_db,
                        },
                        NewFile {
                            position,
                            slot: FileRole::Cover,
                            file: &drawn,
                            name: None,
                        },
                    )
                    .await?;
                    outcome = ThumbnailChange::Redrawn(Box::new(drawn));
                }
            }
        }
        touch(&mut tx, org_db, product_db, at_db).await?;
        tx.commit().await?;
        Ok(Ok(outcome))
    }

    /// Marks a product deleted without erasing it. Every catalogue read
    /// already filters on `deleted_at`, so this is the whole local removal.
    ///
    /// `false` means the tenant has no live product of that identifier, which
    /// makes a repeated delete a no-op rather than a fault.
    pub async fn soft_delete(
        &self,
        org: OrgId,
        id: ProductId,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let deleted = sqlx::query!(
            "UPDATE product SET deleted_at = $3, updated_at = $3 \
             WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
            uuid_to_db(org.0),
            uuid_to_db(id.0),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();

        // A deleted item carries no labels, because the catalogue no longer
        // shows it: leaving the attachments would keep a label alive that no
        // visible item carries, and the board's filter would offer a word
        // whose page is always empty. Cleared in this transaction rather than
        // a later sweep, so the two facts never disagree.
        if deleted == 1 {
            let abandoned = sqlx::query_scalar!(
                "DELETE FROM product_label WHERE org_id = $1 AND product_id = $2 \
                 RETURNING label_id",
                uuid_to_db(org.0),
                uuid_to_db(id.0),
            )
            .fetch_all(&mut *tx)
            .await?;
            crate::labels::sweep_abandoned(&mut tx, org, &abandoned).await?;
        }

        tx.commit().await?;
        Ok(deleted == 1)
    }

    /// Brings back a product a merge tombstoned, inside the reversal window.
    ///
    /// The mirror of [`Self::soft_delete`] and deliberately not its exact
    /// inverse: the delete also detached this product's labels, because a
    /// catalogue no longer showing an item must not keep a label alive that
    /// nothing visible carries. Those attachments are gone and this does not
    /// invent them. What a reversal owes the seller is the resource, and the
    /// caller re-attaches the marketplace label it knows about; a seller's own
    /// labels on a merged-away product are the cost of the merge, and the
    /// thirty-day window is what makes that cost bounded rather than hidden.
    ///
    /// `false` means the tenant has no deleted product of that identifier,
    /// which makes a repeated undo a no-op rather than a fault.
    pub async fn restore(
        &self,
        org: OrgId,
        id: ProductId,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let restored = sqlx::query!(
            "UPDATE product SET deleted_at = NULL, updated_at = $3 \
             WHERE org_id = $1 AND id = $2 AND deleted_at IS NOT NULL",
            uuid_to_db(org.0),
            uuid_to_db(id.0),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(restored == 1)
    }

    /// How many live products this tenant holds, which is what
    /// `TierQuota::listings_max` bounds.
    pub async fn live_count(&self, org: OrgId) -> Result<i64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let counted = sqlx::query_scalar!(
            r#"SELECT count(*) AS "counted!" FROM product
               WHERE org_id = $1 AND deleted_at IS NULL"#,
            uuid_to_db(org.0),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(counted)
    }

    /// How many bytes this tenant's blobs occupy, which is what
    /// `TierQuota::storage_bytes_max` bounds.
    ///
    /// Counted over `blob` rather than over `product_file`: dedup is per
    /// tenant on the hash, so one blob referenced by three products occupies
    /// its bytes once and charging for it three times would bill a seller for
    /// storage nobody uses. A soft-deleted product's blob is still stored, so
    /// it still counts.
    pub async fn stored_bytes(&self, org: OrgId) -> Result<i64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let total = sqlx::query_scalar!(
            r#"SELECT COALESCE(sum(byte_len), 0)::bigint AS "total!" FROM blob WHERE org_id = $1"#,
            uuid_to_db(org.0),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(total)
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

    /// What decides whether a new listing can be created for these resources
    /// on one marketplace, read once for the whole selection.
    ///
    /// Facts and no policy: this reports what the catalogue holds and the
    /// caller decides what the target requires of it, because requiredness is
    /// the registry's answer and a copy of it here would be a second one.
    ///
    /// Batched deliberately. The three questions are asked of forty ticked
    /// resources at a time by a preview, and the per-product reads that would
    /// otherwise answer them — [`Self::get`] and `ElectionRepo::answered_for`
    /// — are a transaction each.
    ///
    /// This replaced a read that answered only the first question. A product
    /// with no payload yields no seed from `mapping_seeds`, so it is silently
    /// absent from the items a create job carries, and a preview has to be
    /// able to say so; the other two are the same kind of silence, one step
    /// further on — bytes nobody can fetch, and a field the target refuses.
    pub async fn creation_facts(
        &self,
        org: OrgId,
        products: &[ProductId],
        inventory: InventoryId,
    ) -> Result<Vec<ProductCreationFacts>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let ids: Vec<uuid::Uuid> = products
            .iter()
            .map(|product| uuid_to_db(product.0))
            .collect();
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let carried = sqlx::query!(
            "SELECT id, rights_state FROM product \
             WHERE org_id = $1 AND id = ANY($2) AND deleted_at IS NULL",
            org_db,
            &ids,
        )
        .fetch_all(&mut *tx)
        .await?;
        let files = sqlx::query!(
            "SELECT product_id, source_marketplace FROM product_file \
             WHERE org_id = $1 AND product_id = ANY($2) \
               AND role = 'payload' AND deleted_at IS NULL",
            org_db,
            &ids,
        )
        .fetch_all(&mut *tx)
        .await?;
        let answered = sqlx::query!(
            "SELECT product_id, axis FROM election_item \
             WHERE org_id = $1 AND product_id = ANY($2) \
               AND inventory = $3 AND state = 'answered'",
            org_db,
            &ids,
            inventory_to_db(inventory),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let mut facts: Vec<ProductCreationFacts> = carried
            .into_iter()
            .map(|row| {
                Ok(ProductCreationFacts {
                    product: ProductId(uuid_from_db(row.id)),
                    // The two states migration 0003 admits. Anything else is a
                    // corrupt row rather than a resource we guess about, which
                    // is the position `rights_from_db` takes on the same
                    // column.
                    rights_declared: match row.rights_state.as_str() {
                        "declared" => true,
                        "unstated" => false,
                        state => {
                            return Err(StorageError::CorruptRow {
                                reason: format!("unknown rights state {state:?}"),
                            })
                        }
                    },
                    payload_files: 0,
                    payload_sources: Vec::new(),
                    settled_axes: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;

        for row in files {
            let product = ProductId(uuid_from_db(row.product_id));
            let Some(held) = facts.iter_mut().find(|facts| facts.product == product) else {
                continue;
            };
            held.payload_files = held.payload_files.saturating_add(1);
            if let Some(raw) = row.source_marketplace.as_deref() {
                let marketplace = crate::connections::marketplace_from_db(raw)?;
                if !held.payload_sources.contains(&marketplace) {
                    held.payload_sources.push(marketplace);
                }
            }
        }
        for row in answered {
            let product = ProductId(uuid_from_db(row.product_id));
            let Some(held) = facts.iter_mut().find(|facts| facts.product == product) else {
                continue;
            };
            let axis = term_kind_from_db(&row.axis)?;
            if !held.settled_axes.contains(&axis) {
                held.settled_axes.push(axis);
            }
        }
        Ok(facts)
    }
}

/// What the seller called this file, where the write carried a name for it.
fn named_as<'a, S: std::hash::BuildHasher>(
    names: &'a HashMap<FileId, String, S>,
    file: &ProductFile,
) -> Option<&'a str> {
    // Only the blob-backed arm may carry one: a sourced row's own name is
    // `payload_file_name`, which migration 0052 governs.
    if matches!(file.bytes, tam_types::FileBytes::Sourced { .. }) {
        return None;
    }
    names.get(&file.id).map(String::as_str)
}

struct FileWrite {
    org: uuid::Uuid,
    product: uuid::Uuid,
    at: DateTime<Utc>,
}

/// Takes the product's row for the length of the transaction, so two file
/// mutations against one product cannot interleave. `false` is the tenant
/// holding no live product of that identifier.
async fn lock_product(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    product_db: uuid::Uuid,
) -> Result<bool, StorageError> {
    let found = sqlx::query_scalar!(
        "SELECT id FROM product \
         WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL \
         FOR UPDATE",
        org_db,
        product_db,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(found.is_some())
}

/// The position a new row takes: one past the greatest this product has ever
/// used, live or retired, because `product_file_position` counts both.
async fn next_position(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    product_db: uuid::Uuid,
) -> Result<i32, StorageError> {
    let highest = sqlx::query_scalar!(
        "SELECT MAX(position) FROM product_file WHERE org_id = $1 AND product_id = $2",
        org_db,
        product_db,
    )
    .fetch_one(&mut **tx)
    .await?;
    highest
        .unwrap_or(-1)
        .checked_add(1)
        .ok_or_else(|| StorageError::Inconsistent {
            reason: "this product has used every file position the column holds".to_owned(),
        })
}

async fn live_role(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    product_db: uuid::Uuid,
    file_db: uuid::Uuid,
) -> Result<Option<FileRole>, StorageError> {
    let row = sqlx::query_scalar!(
        "SELECT role FROM product_file \
         WHERE org_id = $1 AND product_id = $2 AND id = $3 AND deleted_at IS NULL",
        org_db,
        product_db,
        file_db,
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|role| file_role_from_db(&role)).transpose()
}

/// The product's live cover, where it has one.
async fn live_cover(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    product_db: uuid::Uuid,
) -> Result<Option<uuid::Uuid>, StorageError> {
    Ok(sqlx::query_scalar!(
        "SELECT id FROM product_file \
         WHERE org_id = $1 AND product_id = $2 AND role = 'cover' AND deleted_at IS NULL",
        org_db,
        product_db,
    )
    .fetch_optional(&mut **tx)
    .await?)
}

/// The payload file a cover would have been drawn from: the live one with the
/// lowest position, which is the order every read of the product returns them
/// in and the order `ingest` wrote them in.
async fn first_payload(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    product_db: uuid::Uuid,
) -> Result<Option<uuid::Uuid>, StorageError> {
    Ok(sqlx::query_scalar!(
        "SELECT id FROM product_file \
         WHERE org_id = $1 AND product_id = $2 AND role = 'payload' AND deleted_at IS NULL \
         ORDER BY position \
         LIMIT 1",
        org_db,
        product_db,
    )
    .fetch_optional(&mut **tx)
    .await?)
}

/// Whether any marketplace carries this product, which is what migration 0061
/// makes the payload requirement a property of.
async fn has_mapping(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    product_db: uuid::Uuid,
) -> Result<bool, StorageError> {
    let found = sqlx::query_scalar!(
        "SELECT id FROM mapping WHERE org_id = $1 AND product_id = $2 LIMIT 1",
        org_db,
        product_db,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(found.is_some())
}

async fn live_payloads(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    product_db: uuid::Uuid,
) -> Result<i64, StorageError> {
    let counted = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM product_file \
         WHERE org_id = $1 AND product_id = $2 AND role = 'payload' AND deleted_at IS NULL",
        org_db,
        product_db,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(counted.unwrap_or(0))
}

async fn retire(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    product_db: uuid::Uuid,
    file_db: uuid::Uuid,
    at_db: DateTime<Utc>,
) -> Result<(), StorageError> {
    sqlx::query!(
        "UPDATE product_file SET deleted_at = $4 \
         WHERE org_id = $1 AND product_id = $2 AND id = $3 AND deleted_at IS NULL",
        org_db,
        product_db,
        file_db,
        at_db,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Moves the product's own `updated_at`, because a file is part of the
/// resource and a console reporting when it last changed would otherwise miss
/// every change to what a buyer actually downloads. It also re-runs the
/// deferred payload trigger, which fires on `UPDATE product`.
async fn touch(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    product_db: uuid::Uuid,
    at_db: DateTime<Utc>,
) -> Result<(), StorageError> {
    sqlx::query!(
        "UPDATE product SET updated_at = $3 \
         WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
        org_db,
        product_db,
        at_db,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Writes a marketplace-sourced file and its first observation.
///
/// One statement for the row, because the exactly-one CHECK is per statement,
/// and the observation appended in the same transaction so the history holds
/// every observation including the one the file's own columns record.
///
/// There is no blob to compare a length against here, which is the whole
/// difference: the blob-backed branch asserts `blob.byte_len` matches the
/// file's before it writes, and that assertion is the only thing standing
/// between a mismatched length and a manifest that lies to a device. Nothing
/// on this branch can make the same promise, so it does not pretend to — the
/// length is the device's report and is recorded as such.
async fn insert_sourced_file(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    write: &FileWrite,
    position: i32,
    file: &ProductFile,
) -> Result<(), StorageError> {
    let FileWrite {
        org: org_db,
        product: product_db,
        at: at_db,
    } = *write;
    let tam_types::FileBytes::Sourced {
        marketplace,
        connection,
        resource,
        entry,
        payload_file_name,
        payload_content_type,
        observed,
    } = &file.bytes
    else {
        return Err(StorageError::Inconsistent {
            reason: "insert_sourced_file called for a file that holds its own bytes".to_owned(),
        });
    };
    let asserted = ScanColumns::from_outcome(&observed.scan)?;
    let observed_byte_len =
        i64::try_from(observed.byte_len).map_err(|_| StorageError::Inconsistent {
            reason: format!(
                "observed byte length {} exceeds the column range",
                observed.byte_len
            ),
        })?;
    sqlx::query!(
        "INSERT INTO product_file \
         (org_id, id, product_id, position, role, kind, created_at, \
          source_marketplace, source_connection, source_resource, source_entry, \
          observed_hash, observed_byte_len, \
          asserted_scan_state, asserted_scan_signature, asserted_scan_failure_code, \
          asserted_scanned_at, observed_by_device, observed_at, recorded_at, \
          payload_file_name, payload_content_type) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                 $14, $15, $16, $17, $18, $19, $20, $21, $22)",
        org_db,
        uuid_to_db(file.id.0),
        product_db,
        position,
        file_role_to_db(file.role),
        file_kind_to_db(file.kind),
        at_db,
        crate::codec::marketplace_to_db(*marketplace),
        uuid_to_db(connection.0),
        resource,
        entry.as_deref(),
        hash_to_db(observed.hash),
        observed_byte_len,
        asserted.state,
        asserted.signature,
        asserted.failure_code,
        asserted.scanned_at,
        observed.device,
        timestamp_to_db(observed.observed_at)?,
        at_db,
        payload_file_name,
        payload_content_type,
    )
    .execute(&mut **tx)
    .await?;
    crate::file_source::append(
        tx,
        tam_types::OrgId(crate::codec::uuid_from_db(org_db)),
        file.id,
        observed,
        crate::codec::timestamp_from_db(at_db),
    )
    .await
}

/// One file as a write states it: where it sits, which slot it fills, the file
/// itself, and what the seller called it.
struct NewFile<'a> {
    position: i32,
    slot: FileRole,
    file: &'a ProductFile,
    /// Absent for a file nobody named, which is every file written before the
    /// name column existed and every cover, which no seller chooses.
    name: Option<&'a str>,
}

async fn insert_file(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    write: &FileWrite,
    new: NewFile<'_>,
) -> Result<(), StorageError> {
    let NewFile {
        position,
        slot,
        file,
        name,
    } = new;
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
    // The two arms are two different statements rather than one with more
    // columns, because `product_file_blob_or_source` is checked per statement:
    // a sourced row has to arrive with its whole group, and a blob-backed one
    // has to arrive with none of it.
    let (hash, byte_len, scan) = match &file.bytes {
        tam_types::FileBytes::Held {
            hash,
            byte_len,
            scan,
        } => (*hash, *byte_len, scan),
        tam_types::FileBytes::Sourced { .. } => {
            // `product_file_name_is_blob_backed` refuses a name here, and
            // 0052's `payload_file_name` is this arm's own answer to the same
            // question. A caller offering one has confused the two arms.
            if name.is_some() {
                return Err(StorageError::Inconsistent {
                    reason: "a marketplace-sourced file was given a seller's filename".to_owned(),
                });
            }
            return insert_sourced_file(tx, write, position, file).await;
        }
    };
    let hash = hash_to_db(hash);
    let byte_len = i64::try_from(byte_len).map_err(|_| StorageError::Inconsistent {
        reason: format!("byte length {byte_len} exceeds the column range"),
    })?;
    let object_key = format!("blob/{}", hash_hex(crate::codec::hash_from_db(&hash)?));

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

    let scan = ScanColumns::from_outcome(scan)?;
    sqlx::query!(
        "INSERT INTO product_file \
         (org_id, id, product_id, position, role, kind, hash, \
          scan_state, scan_signature, scanned_at, scan_failure_code, created_at, name) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
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
        name,
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
    hash: Option<Vec<u8>>,
    byte_len: Option<i64>,
    scan_state: Option<String>,
    scan_signature: Option<String>,
    scanned_at: Option<DateTime<Utc>>,
    scan_failure_code: Option<String>,
    source_marketplace: Option<String>,
    source_connection: Option<uuid::Uuid>,
    source_resource: Option<String>,
    source_entry: Option<String>,
    observed_hash: Option<Vec<u8>>,
    observed_byte_len: Option<i64>,
    observed_by_device: Option<String>,
    observed_at: Option<DateTime<Utc>>,
    asserted_scan_state: Option<String>,
    asserted_scan_signature: Option<String>,
    asserted_scanned_at: Option<DateTime<Utc>>,
    asserted_scan_failure_code: Option<String>,
    payload_file_name: Option<String>,
    payload_content_type: Option<String>,
    name: Option<String>,
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

/// One row into a file, choosing its arm by whether the server holds a digest.
///
/// `product_file_blob_or_source` is what makes the choice total: a row has a
/// hash and no source group, or a source group and no hash, so there is no
/// third case to represent and none to guess at. Every `missing` below names
/// the constraint that was supposed to prevent it, because reaching one means
/// the row is corrupt rather than that the branch needs handling.
fn decode_file(row: FileRow) -> Result<(FileRole, ProductFile, Option<String>), StorageError> {
    let name = row.name.clone();
    let role = file_role_from_db(&row.role)?;
    let bytes = match row.hash {
        Some(hash) => tam_types::FileBytes::Held {
            hash: hash_from_db(&hash)?,
            byte_len: u64::try_from(missing(row.byte_len, "byte_len")?).map_err(|_| {
                StorageError::CorruptRow {
                    reason: "negative blob byte_len".to_owned(),
                }
            })?,
            scan: scan_from_db(
                &missing(row.scan_state, "scan_state")?,
                row.scan_signature,
                row.scanned_at,
                row.scan_failure_code.as_deref(),
            )?,
        },
        None => tam_types::FileBytes::Sourced {
            marketplace: crate::connections::marketplace_from_db(&missing(
                row.source_marketplace,
                "source_marketplace",
            )?)?,
            connection: tam_types::ConnectionId(uuid_from_db(missing(
                row.source_connection,
                "source_connection",
            )?)),
            resource: missing(row.source_resource, "source_resource")?,
            entry: row.source_entry,
            payload_file_name: missing(row.payload_file_name, "payload_file_name")?,
            payload_content_type: missing(row.payload_content_type, "payload_content_type")?,
            observed: tam_types::Observation {
                device: missing(row.observed_by_device, "observed_by_device")?,
                hash: hash_from_db(&missing(row.observed_hash, "observed_hash")?)?,
                byte_len: u64::try_from(missing(row.observed_byte_len, "observed_byte_len")?)
                    .map_err(|_| StorageError::CorruptRow {
                        reason: "negative observed byte_len".to_owned(),
                    })?,
                scan: scan_from_db(
                    &missing(row.asserted_scan_state, "asserted_scan_state")?,
                    row.asserted_scan_signature,
                    row.asserted_scanned_at,
                    row.asserted_scan_failure_code.as_deref(),
                )?,
                observed_at: crate::codec::timestamp_from_db(missing(
                    row.observed_at,
                    "observed_at",
                )?),
            },
        },
    };
    Ok((
        role,
        ProductFile {
            id: FileId(uuid_from_db(row.id)),
            role,
            kind: file_kind_from_db(&row.kind)?,
            bytes,
        },
        name,
    ))
}

/// A column the exactly-one CHECK guarantees is present on the branch taken.
fn missing<T>(value: Option<T>, column: &str) -> Result<T, StorageError> {
    value.ok_or_else(|| StorageError::CorruptRow {
        reason: format!("{column} is absent on a row product_file_blob_or_source admitted"),
    })
}

/// The files of one product, and what the seller called each of the ones they
/// named.
///
/// The names travel beside the files rather than inside `ProductFile`, which
/// carries no name field: adding one would reach every construction of that
/// type across the workspace, and a name is a fact about a row this repository
/// stores rather than a part of the file the domain reasons about.
pub struct ProductFiles {
    /// None where the product carries no live payload row, which is a resource
    /// kept on Teachouse rather than a corrupt product (D32).
    pub payload: Option<PayloadSet>,
    pub cover: Option<ProductFile>,
    pub previews: Vec<ProductFile>,
    pub names: HashMap<FileId, String>,
}

fn partition_files(rows: Vec<FileRow>) -> Result<ProductFiles, StorageError> {
    let mut payload = Vec::new();
    let mut cover = None;
    let mut previews = Vec::new();
    let mut names = HashMap::new();
    for row in rows {
        let (role, file, name) = decode_file(row)?;
        if let Some(name) = name {
            names.insert(file.id, name);
        }
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
    // No live payload row is now a resource kept on Teachouse rather than a
    // corrupt one (D32): the trigger that made it corrupt raises only for a
    // product a mapping names, so a row without one is a draft nobody has
    // pointed anywhere yet and reads back as carrying no file.
    let mut payload = payload.into_iter();
    let held = payload
        .next()
        .map(|head| PayloadSet::new(head, payload.collect()));
    Ok(ProductFiles {
        payload: held,
        cover,
        previews,
        names,
    })
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

/// One product's cover, as the catalogue and the cover route read it.
#[derive(Debug, Clone, Copy)]
pub struct StoredCover {
    pub product: ProductId,
    pub hash: ContentHash,
}

impl ProductRepo {
    /// The catalogue page: keyset on `(created_at, id)` strictly above the
    /// cursor, oldest first, matching the unpaginated listing's order.
    /// One keyset page of the catalogue, optionally narrowed to the items
    /// carrying one label.
    ///
    /// The label is a clause in the page query rather than a filter over the
    /// page it returns: filtering afterwards would shorten pages below the
    /// limit and leave the cursor pointing past items the caller never saw.
    /// Compared case-insensitively, matching `label_one_per_name`, so a filter
    /// finds the label whatever capitalisation reaches it.
    pub async fn list_page(
        &self,
        org: OrgId,
        cursor: Option<crate::job_reads::LedgerCursor>,
        limit: i64,
        label: Option<&str>,
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
               AND ($5::text IS NULL OR EXISTS ( \
                     SELECT 1 FROM product_label pl \
                     JOIN label l ON l.org_id = pl.org_id AND l.id = pl.label_id \
                     WHERE pl.org_id = product.org_id AND pl.product_id = product.id \
                       AND lower(l.name) = lower($5))) \
             ORDER BY created_at, id LIMIT $4",
            org_db,
            cursor_at,
            cursor_id,
            limit,
            label,
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

    /// The stored cover of each of these products.
    ///
    /// Only a cover whose bytes this deployment holds. A `product_file` row
    /// may name a marketplace resource instead of a blob, and a row like that
    /// has nothing here to serve, so it is absent rather than listed with a
    /// hash it does not have.
    ///
    /// A deleted resource has no cover. The join is what says so: a delete is
    /// a tombstone on `product` and leaves its file rows alone, so a query
    /// reading `product_file` on its own would go on serving the picture of a
    /// resource the catalogue no longer lists.
    ///
    /// One statement for the whole page, keyed by the page's product
    /// identifiers, exactly as [`Self::export_page`]'s listings read is.
    pub async fn covers(
        &self,
        org: OrgId,
        products: &[ProductId],
    ) -> Result<Vec<StoredCover>, StorageError> {
        if products.is_empty() {
            return Ok(Vec::new());
        }
        let org_db = uuid_to_db(org.0);
        let ids: Vec<uuid::Uuid> = products
            .iter()
            .map(|product| uuid_to_db(product.0))
            .collect();
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT pf.product_id, pf.hash FROM product_file pf \
             JOIN product p ON p.org_id = pf.org_id AND p.id = pf.product_id \
             WHERE pf.org_id = $1 AND pf.product_id = ANY($2::uuid[]) \
               AND pf.role = 'cover' AND pf.deleted_at IS NULL AND pf.hash IS NOT NULL \
               AND p.deleted_at IS NULL",
            org_db,
            &ids,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                let hash = row.hash.ok_or_else(|| StorageError::CorruptRow {
                    reason: "a cover row selected on hash IS NOT NULL carried no hash".to_owned(),
                })?;
                Ok(StoredCover {
                    product: ProductId(uuid_from_db(row.product_id)),
                    hash: hash_from_db(&hash)?,
                })
            })
            .collect()
    }

    /// One page of the catalogue as an export reads it, keyset-walked by
    /// `(created_at, id)` exactly as [`Self::list_page`] is.
    ///
    /// Two statements for the whole page rather than two transactions for
    /// every resource in it: the labels come back as an array beside their
    /// product, and every listing on the page in one read keyed by the page's
    /// product identifiers.
    pub async fn export_page(
        &self,
        org: OrgId,
        cursor: Option<crate::job_reads::LedgerCursor>,
        limit: i64,
    ) -> Result<Vec<ExportedResource>, StorageError> {
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
            ExportProductRow,
            "SELECT id, title, price_kind, price_minor_units, price_currency, \
             created_at, updated_at, \
             ARRAY(SELECT l.name FROM product_label pl \
                     JOIN label l ON l.org_id = pl.org_id AND l.id = pl.label_id \
                    WHERE pl.org_id = product.org_id AND pl.product_id = product.id \
                    ORDER BY lower(l.name)) AS \"labels!\" \
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
        if rows.is_empty() {
            tx.commit().await?;
            return Ok(vec![]);
        }
        let page: Vec<uuid::Uuid> = rows.iter().map(|row| row.id).collect();
        let listings = sqlx::query_as!(
            ExportListingRow,
            "SELECT product_id, inventory, binding_state, lifecycle_state, \
             remote_id_kind, remote_url, remote_numeric_id, \
             price_rule_kind, price_explicit_kind, price_explicit_minor_units, \
             price_explicit_currency \
             FROM mapping WHERE org_id = $1 AND product_id = ANY($2) \
             ORDER BY product_id, inventory",
            org_db,
            &page[..],
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        decode_export(rows, listings)
    }
}

struct ExportProductRow {
    id: uuid::Uuid,
    title: String,
    price_kind: String,
    price_minor_units: Option<i64>,
    price_currency: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    labels: Vec<String>,
}

struct ExportListingRow {
    product_id: uuid::Uuid,
    inventory: String,
    binding_state: String,
    lifecycle_state: String,
    remote_id_kind: Option<String>,
    remote_url: Option<String>,
    remote_numeric_id: Option<i64>,
    price_rule_kind: String,
    price_explicit_kind: Option<String>,
    price_explicit_minor_units: Option<i64>,
    price_explicit_currency: Option<String>,
}

/// Joins the page's listings onto their products in memory, which is where
/// that join belongs: reading them as one statement is the point of the second
/// query, and the page is already bounded by its own limit.
fn decode_export(
    rows: Vec<ExportProductRow>,
    listings: Vec<ExportListingRow>,
) -> Result<Vec<ExportedResource>, StorageError> {
    let mut by_product: HashMap<uuid::Uuid, Vec<ExportedListing>> = HashMap::new();
    for listing in listings {
        let product = listing.product_id;
        by_product
            .entry(product)
            .or_default()
            .push(decode_listing(listing)?);
    }
    rows.into_iter()
        .map(|row| {
            Ok(ExportedResource {
                id: ProductId(uuid_from_db(row.id)),
                title: Title(row.title),
                price: price_from_db(&row.price_kind, row.price_minor_units, row.price_currency)?,
                labels: row.labels,
                created_at: timestamp_from_db(row.created_at),
                updated_at: timestamp_from_db(row.updated_at),
                listings: by_product.remove(&row.id).unwrap_or_default(),
            })
        })
        .collect()
}

fn decode_listing(row: ExportListingRow) -> Result<ExportedListing, StorageError> {
    let remote = if row.binding_state == "bound" {
        let kind = row
            .remote_id_kind
            .as_deref()
            .ok_or_else(|| StorageError::CorruptRow {
                reason: format!(
                    "bound mapping on product {} carries no remote id kind",
                    row.product_id
                ),
            })?;
        Some(remote_id_from_db(
            kind,
            row.remote_url,
            row.remote_numeric_id,
        )?)
    } else {
        None
    };
    let listed_price = if row.price_rule_kind == "explicit" {
        let kind = row
            .price_explicit_kind
            .as_deref()
            .ok_or_else(|| StorageError::CorruptRow {
                reason: "explicit price rule without a price kind".to_owned(),
            })?;
        Some(price_from_db(
            kind,
            row.price_explicit_minor_units,
            row.price_explicit_currency,
        )?)
    } else {
        None
    };
    Ok(ExportedListing {
        inventory: inventory_from_db(&row.inventory)?,
        binding_state: row.binding_state,
        lifecycle_state: row.lifecycle_state,
        remote,
        listed_price,
    })
}

/// Writes a product inside a transaction the caller owns.
///
/// The one implementation of the catalogue insert, and the reason it is
/// reachable this way is the import's commit: the product, its source
/// binding, its sketch and the import row's outcome are one decision, and a
/// product written by its own transaction is a product that survives a
/// rolled-back decision. [`ProductRepo::insert_named`] is this call with a
/// transaction opened around it, which is what every standalone caller wants.
pub async fn insert_product<S: std::hash::BuildHasher>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    product: &CanonicalProduct,
    names: &HashMap<FileId, String, S>,
    at: Timestamp,
) -> Result<(), StorageError> {
    if product.org != org {
        return Err(StorageError::OrgMismatch);
    }
    let org_db = uuid_to_db(org.0);
    let product_db = uuid_to_db(product.id.0);
    let at_db = timestamp_to_db(at)?;

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
    .execute(&mut **tx)
    .await?;

    let write = FileWrite {
        org: org_db,
        product: product_db,
        at: at_db,
    };
    let mut position: i32 = 0;
    for file in product.payload_files() {
        let downloaded = NewFile {
            position,
            slot: FileRole::Payload,
            file,
            name: named_as(names, file),
        };
        insert_file(tx, &write, downloaded).await?;
        position += 1;
    }
    if let Some(cover) = &product.cover {
        // No name: a cover is generated from the first payload's bytes
        // rather than chosen, so there is nothing a seller called it.
        let drawn = NewFile {
            position,
            slot: FileRole::Cover,
            file: cover,
            name: None,
        };
        insert_file(tx, &write, drawn).await?;
        position += 1;
    }
    for preview in &product.previews {
        let shown = NewFile {
            position,
            slot: FileRole::Preview,
            file: preview,
            name: named_as(names, preview),
        };
        insert_file(tx, &write, shown).await?;
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
        .execute(&mut **tx)
        .await?;
    }

    insert_grades(tx, org_db, product_db, &product.grades).await?;

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
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// One product's title, read inside a transaction the caller owns.
///
/// The import's commit needs it for a sentence the seller reads — "same as
/// Fractions pack" — about a product it has just decided against creating a
/// second copy of. Read under the same lock as the decision rather than
/// before it, because before the lock nobody knows which product the answer
/// will be about.
pub async fn title_of(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    product: ProductId,
) -> Result<Option<String>, StorageError> {
    let row = sqlx::query!(
        "SELECT title FROM product WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
        uuid_to_db(org.0),
        uuid_to_db(product.0),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| row.title))
}

/// Edits one product inside a transaction the caller owns.
///
/// The one implementation. The duplicate review's merge writes the survivor's
/// winning fields, tombstones the loser and records the verdict as one
/// decision, so a process that stops between them cannot leave a tombstoned
/// resource beside a question that still reads as unanswered.
pub async fn update_product(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    id: ProductId,
    edit: &ProductEdit,
    at: Timestamp,
) -> Result<bool, StorageError> {
    let org_db = uuid_to_db(org.0);
    let product_db = uuid_to_db(id.0);
    let at_db = timestamp_to_db(at)?;
    let price = edit.price.map(PriceColumns::from_intent);
    let rights = edit.rights.as_ref().map(RightsColumns::encode);

    let touched = sqlx::query!(
        "UPDATE product SET \
         title = COALESCE($3, title), \
         body = COALESCE($4, body), \
         body_format = COALESCE($5, body_format), \
         price_kind = COALESCE($6, price_kind), \
         price_minor_units = CASE WHEN $6 IS NULL THEN price_minor_units ELSE $7 END, \
         price_currency = CASE WHEN $6 IS NULL THEN price_currency ELSE $8 END, \
         rights_state = COALESCE($9, rights_state), \
         rights_source_inventory = \
             CASE WHEN $9 IS NULL THEN rights_source_inventory ELSE $10 END, \
         rights_segments = CASE WHEN $9 IS NULL THEN rights_segments ELSE $11 END, \
         rights_native_id = CASE WHEN $9 IS NULL THEN rights_native_id ELSE $12 END, \
         updated_at = $13 \
         WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
        org_db,
        product_db,
        edit.title.as_ref().map(|title| title.0.as_str()),
        edit.body.as_ref().map(|copy| copy.body.as_str()),
        edit.body
            .as_ref()
            .map(|copy| copy_format_to_db(copy.format)),
        price.as_ref().map(|price| price.kind),
        price.as_ref().and_then(|price| price.minor_units),
        price.as_ref().and_then(|price| price.currency),
        rights.as_ref().map(|rights| rights.state),
        rights.as_ref().and_then(|rights| rights.inventory.clone()),
        rights
            .as_ref()
            .and_then(|rights| rights.segments.as_deref()),
        rights
            .as_ref()
            .and_then(|rights| rights.native_id.as_deref()),
        at_db,
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if touched == 0 {
        return Ok(false);
    }

    if let Some(subjects) = &edit.subjects {
        sqlx::query!(
            "DELETE FROM product_term WHERE org_id = $1 AND product_id = $2",
            org_db,
            product_db,
        )
        .execute(&mut **tx)
        .await?;
        for (index, term) in subjects.iter().enumerate() {
            let position = i32::try_from(index).map_err(|_| StorageError::Inconsistent {
                reason: format!("subject position {index} exceeds the column range"),
            })?;
            sqlx::query!(
                "INSERT INTO product_term (org_id, product_id, term_id, position) \
                 VALUES ($1, $2, $3, $4)",
                org_db,
                product_db,
                uuid_to_db(term.0),
                position,
            )
            .execute(&mut **tx)
            .await?;
        }
    }

    if let Some(grades) = &edit.grades {
        // grade_declaration_path cascades off the declaration row, so one
        // delete clears both and the reinsert is the whole replacement.
        sqlx::query!(
            "DELETE FROM grade_declaration WHERE org_id = $1 AND product_id = $2",
            org_db,
            product_db,
        )
        .execute(&mut **tx)
        .await?;
        insert_grades(tx, org_db, product_db, grades).await?;
    }
    Ok(true)
}

/// Tombstones one product inside a transaction the caller owns.
///
/// A tombstone rather than an erasure, which is what makes the thirty-day
/// reversal possible; reachable here so the merge that decided it and the
/// verdict recording that decision are one transaction.
pub async fn soft_delete_product(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    id: ProductId,
    at: Timestamp,
) -> Result<bool, StorageError> {
    let deleted = sqlx::query!(
        "UPDATE product SET deleted_at = $3, updated_at = $3 \
         WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
        uuid_to_db(org.0),
        uuid_to_db(id.0),
        timestamp_to_db(at)?,
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(deleted > 0)
}

/// Brings a tombstoned product back inside a transaction the caller owns.
pub async fn restore_product(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    id: ProductId,
    at: Timestamp,
) -> Result<bool, StorageError> {
    let restored = sqlx::query!(
        "UPDATE product SET deleted_at = NULL, updated_at = $3 \
         WHERE org_id = $1 AND id = $2 AND deleted_at IS NOT NULL",
        uuid_to_db(org.0),
        uuid_to_db(id.0),
        timestamp_to_db(at)?,
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(restored > 0)
}
