//! The catalogue import: the seller's own marketplace listing, read under
//! the first-party-export capability, canonicalised into a product with its
//! files ingested, its taxonomy mapped inbound over the seeded crosswalk,
//! its grade declaration retained verbatim, and its target mapping created —
//! then projected once, so every gap raises its queue item on the spot and
//! the drain report the kill gate reads falls out of the run itself.
//!
//! Customer zero's file bytes arrive from disk (the founder has the
//! originals); the marketplace file-download leg is an uncaptured endpoint
//! and ships only after a supervised capture, per the plan.

#![forbid(unsafe_code)]

use sqlx::PgPool;
use tam_marketplace::transport::Transport;
use tam_marketplace::{AdapterError, FetchReason, FileContent, FileSource, FileSourceError};
use tam_marketplace_tes::{DraftId, TesAdapter};
use tam_pipeline::archive::ExtractBudget;
use tam_pipeline::pipeline::{ingest, IngestContext, IngestError, Ingested};
use tam_pipeline::scan::EicarScanner;
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::{
    BlobRepo, MappingRepo, ProductRepo, RaiseReport, RaiseScope, StorageError, TaxonomyRepo,
    TenantBlobSink,
};
use tam_taxonomy::listing::{project_listing, ListingContext};
use tam_taxonomy::project::ingest_by_native_id;
use tam_taxonomy::TES_MAIN_AGE_RANGES;
use tam_types::{
    CanonicalTermId, ContentHash, Currency, FileId, FileKind, FileRole, InventoryId, ListingCopy,
    MappingId, Money, OrgId, PayloadSet, PriceIntent, ProductFile, ProductId, ScanOutcome,
    Timestamp, Title, Uuid,
};

/// The import never uploads, so its adapter's file source is a refusal.
pub struct NoImportFiles;

impl FileSource for NoImportFiles {
    fn fetch(
        &self,
        file: FileId,
    ) -> impl core::future::Future<Output = Result<FileContent, FileSourceError>> + Send {
        core::future::ready(Err(FileSourceError::Unreadable {
            file,
            detail: "the import reads listings and never uploads files".to_owned(),
        }))
    }
}

/// One manifest row: the seller's resource and its file bytes from disk.
pub struct ImportEntry {
    pub resource: i64,
    pub files: Vec<NamedBytes>,
}

pub struct NamedBytes {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Everything an import run holds constant across entries.
pub struct ImportRun<'a, T: Transport> {
    pub pool: PgPool,
    pub kek: Kek,
    pub store_root: std::path::PathBuf,
    pub adapter: &'a TesAdapter<T, NoImportFiles>,
    pub org: OrgId,
    pub source: InventoryId,
    pub target: InventoryId,
    pub now: Timestamp,
}

/// What one entry became, and what the drain learned from it.
#[derive(Debug)]
pub struct ImportRowReport {
    pub resource: i64,
    pub product: ProductId,
    pub mapping: MappingId,
    pub title: String,
    pub terms_seen: usize,
    pub terms_mapped: usize,
    pub unmapped_native_ids: Vec<String>,
    pub curriculum: Vec<String>,
    pub raised: RaiseReport,
    pub projectable: bool,
    pub blocked_by: Option<String>,
}

#[derive(Debug)]
pub enum ImportError {
    Adapter(AdapterError),
    Ingest(IngestError),
    Storage(StorageError),
    NoPayload,
    Price(String),
}

impl core::fmt::Display for ImportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Adapter(error) => write!(f, "adapter: {error:?}"),
            Self::Ingest(error) => write!(f, "ingest: {error}"),
            Self::Storage(error) => write!(f, "storage: {error}"),
            Self::NoPayload => f.write_str("the entry carried no ingestable payload"),
            Self::Price(detail) => write!(f, "price: {detail}"),
        }
    }
}

impl core::error::Error for ImportError {}

impl From<AdapterError> for ImportError {
    fn from(error: AdapterError) -> Self {
        Self::Adapter(error)
    }
}

impl From<IngestError> for ImportError {
    fn from(error: IngestError) -> Self {
        Self::Ingest(error)
    }
}

impl From<StorageError> for ImportError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

pub async fn import_one<T: Transport>(
    run: &ImportRun<'_, T>,
    entry: &ImportEntry,
) -> Result<ImportRowReport, ImportError> {
    let listing = run
        .adapter
        .fetch_for_import(
            &FetchReason::FirstPartyExport {
                inventory: run.source,
            },
            DraftId(entry.resource),
        )
        .await?;

    // Files through the pipeline: every guard M1f built applies to an import
    // exactly as to an upload. The first entry's cover becomes the product's.
    let blob_repo = BlobRepo::new(
        run.pool.clone(),
        LocalObjectStore::new(run.store_root.clone()),
        run.kek.clone(),
    );
    let sink = TenantBlobSink {
        repo: &blob_repo,
        org: run.org,
        at: run.now,
    };
    let mut payloads: Vec<ProductFile> = Vec::new();
    let mut cover: Option<ProductFile> = None;
    for file in &entry.files {
        let ingested: Ingested = ingest(
            &file.bytes,
            &EicarScanner,
            &sink,
            IngestContext {
                budget: ExtractBudget::default(),
                now: run.now,
            },
        )
        .await?;
        for stored in &ingested.payload {
            payloads.push(ProductFile {
                id: FileId(fresh_uuid()),
                role: FileRole::Payload,
                kind: stored.kind,
                hash: stored.hash,
                byte_len: stored.byte_len,
                scan: ScanOutcome::Clean { at: run.now },
            });
        }
        if cover.is_none() {
            cover = Some(ProductFile {
                id: FileId(fresh_uuid()),
                role: FileRole::Cover,
                kind: FileKind::Image,
                hash: ingested.cover.hash,
                byte_len: ingested.cover.byte_len,
                scan: ScanOutcome::Clean { at: run.now },
            });
        }
    }
    let mut payload_iter = payloads.into_iter();
    let head = payload_iter.next().ok_or(ImportError::NoPayload)?;
    let payload = PayloadSet::new(head, payload_iter.collect());

    // Taxonomy inbound by native id over the source vocabulary's edges. An
    // unmapped id is retained verbatim in the report — the item type cannot
    // name a canonical term that does not exist — and the outbound raise
    // below covers every term that does.
    let taxonomy = TaxonomyRepo::new(run.pool.clone());
    let subject_edges = taxonomy
        .edges_into(tam_domain::VocabularyId(
            run.source,
            tam_domain::TermKind::Subject,
        ))
        .await?;
    let topic_edges = taxonomy
        .edges_into(tam_domain::VocabularyId(
            run.source,
            tam_domain::TermKind::Topic,
        ))
        .await?;
    let mut subjects: Vec<CanonicalTermId> = Vec::new();
    let mut unmapped: Vec<String> = Vec::new();
    for native in &listing.category_native_ids {
        let found = ingest_by_native_id(
            native,
            tam_domain::VocabularyId(run.source, tam_domain::TermKind::Subject),
            &subject_edges,
        )
        .or_else(|| {
            ingest_by_native_id(
                native,
                tam_domain::VocabularyId(run.source, tam_domain::TermKind::Topic),
                &topic_edges,
            )
        });
        match found {
            Some(term) if !subjects.contains(&term) => subjects.push(term),
            Some(_) => {}
            None => unmapped.push(native.clone()),
        }
    }
    let terms_seen = listing.category_native_ids.len();
    let terms_mapped = subjects.len();

    // Grades verbatim: the declared age-range ids as paths, the label looked
    // up from the measured table where it exists, the interval derived only
    // when every declared range is bounded.
    let grade_paths: Vec<tam_domain::VocabularyPath> = listing
        .age_range_native_ids
        .iter()
        .map(|native| {
            let label = TES_MAIN_AGE_RANGES
                .iter()
                .find(|row| row.native_id == native.as_str())
                .map_or_else(|| native.clone(), |row| row.label.to_owned());
            tam_domain::VocabularyPath {
                vocabulary: tam_domain::VocabularyId(run.source, tam_domain::TermKind::Phase),
                segments: vec![label],
                native_id: Some(native.clone()),
            }
        })
        .collect();
    let grades = tam_domain::GradeDeclaration {
        source: tam_domain::DeclarationSource::Imported {
            vocabulary: tam_domain::VocabularyId(run.source, tam_domain::TermKind::Phase),
        },
        derived: tam_taxonomy::derive_interval(&grade_paths),
        raw: grade_paths,
    };

    let price = price_intent(listing.licence.as_deref(), listing.price)?;

    let product_id = ProductId(fresh_uuid());
    let product = tam_domain::CanonicalProduct {
        id: product_id,
        org: run.org,
        title: Title(listing.title.clone()),
        body: ListingCopy {
            body: listing.body.clone(),
        },
        payload,
        cover,
        previews: vec![],
        subjects,
        grades,
        price,
    };
    ProductRepo::new(run.pool.clone())
        .insert(run.org, &product, run.now)
        .await?;

    let mapping_id = MappingId(fresh_uuid());
    MappingRepo::new(run.pool.clone())
        .insert(
            run.org,
            &tam_domain::Mapping {
                id: mapping_id,
                org: run.org,
                product: product_id,
                inventory: run.target,
                binding: tam_domain::Binding::Unbound,
                policies: tam_domain::FieldPolicies {
                    title: tam_domain::FieldPolicy::Managed,
                    description: tam_domain::FieldPolicy::Managed,
                    price: tam_domain::FieldPolicy::Managed,
                    taxonomy: tam_domain::FieldPolicy::Managed,
                    grades: tam_domain::FieldPolicy::Managed,
                    files: tam_domain::FieldPolicy::Managed,
                },
                price_rule: tam_types::PriceRule::Explicit(price),
                publish: tam_domain::PublishMode::DryRun,
                lifecycle: tam_marketplace::RemoteLifecycle::Absent,
            },
            0,
            run.now,
        )
        .await?;

    // Project once, immediately: the gaps raise their items now, which is
    // what makes the import's own report the drain measurement.
    let terms = taxonomy.terms().await?;
    let mut target_edges = taxonomy
        .edges_into(tam_domain::VocabularyId(
            run.target,
            tam_domain::TermKind::Subject,
        ))
        .await?;
    target_edges.extend(
        taxonomy
            .edges_into(tam_domain::VocabularyId(
                run.target,
                tam_domain::TermKind::Topic,
            ))
            .await?,
    );
    let no_counterparts = taxonomy.no_counterparts_into(run.target).await?;
    let outcome = project_listing(
        &product,
        &ListingContext {
            org: run.org,
            mapping: mapping_id,
            inventory: run.target,
            now: run.now,
            terms: &terms,
            edges: &target_edges,
            no_counterparts: &no_counterparts,
        },
    );
    let (projectable, blocked_by, raised) = match outcome {
        Ok(_) => (
            true,
            None,
            RaiseReport {
                new: 0,
                already_open: 0,
            },
        ),
        Err(tam_domain::ProjectionBlocked::Taxonomy { items }) => {
            let causes: Vec<(CanonicalTermId, tam_domain::TermKind)> = items
                .iter()
                .map(|item| {
                    let tam_domain::VocabularyId(_, kind) = item.target;
                    (item.term, kind)
                })
                .collect();
            let raised = taxonomy
                .raise(
                    run.org,
                    RaiseScope {
                        mapping: mapping_id,
                        target: run.target,
                        at: run.now,
                    },
                    &causes,
                )
                .await?;
            (false, Some("taxonomy".to_owned()), raised)
        }
        Err(tam_domain::ProjectionBlocked::CurrencyUnknown { .. }) => (
            false,
            Some("currency_unknown".to_owned()),
            RaiseReport {
                new: 0,
                already_open: 0,
            },
        ),
        Err(tam_domain::ProjectionBlocked::CoverMissing) => (
            false,
            Some("cover_missing".to_owned()),
            RaiseReport {
                new: 0,
                already_open: 0,
            },
        ),
        Err(tam_domain::ProjectionBlocked::ScanIncomplete { .. }) => (
            false,
            Some("scan_incomplete".to_owned()),
            RaiseReport {
                new: 0,
                already_open: 0,
            },
        ),
    };

    Ok(ImportRowReport {
        resource: entry.resource,
        product: product_id,
        mapping: mapping_id,
        title: listing.title,
        terms_seen,
        terms_mapped,
        unmapped_native_ids: unmapped,
        curriculum: listing.curriculum,
        raised,
        projectable,
        blocked_by,
    })
}

/// Tes prices are the licence split the spike measured: a Creative Commons
/// licence is free, `TES-PAID` carries a decimal price in account currency.
fn price_intent(licence: Option<&str>, price: Option<f64>) -> Result<PriceIntent, ImportError> {
    match licence {
        None => Ok(PriceIntent::Free),
        Some(cc) if cc.starts_with("CC-") => Ok(PriceIntent::Free),
        Some("TES-PAID") => {
            let value = price
                .ok_or_else(|| ImportError::Price("TES-PAID without a price value".to_owned()))?;
            let pence = (value * 100.0).round();
            if !(0.0..=1_000_000_000.0).contains(&pence) {
                return Err(ImportError::Price(format!(
                    "price {value} is outside the representable range"
                )));
            }
            #[expect(
                clippy::cast_possible_truncation,
                reason = "rounded and range-checked immediately above; the cast is the conversion"
            )]
            let minor = pence as i64;
            Money::new(minor, Currency::Gbp)
                .map(PriceIntent::Paid)
                .map_err(|error| ImportError::Price(error.to_string()))
        }
        Some(other) => Err(ImportError::Price(format!(
            "unrecognised licence {other:?}; refusing to guess whether this is paid"
        ))),
    }
}

/// Row identity, minted at the import boundary.
fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}

/// The blake3 content hash type is re-exported for the binary's manifest
/// handling; nothing else here is marketplace-shaped.
pub type PayloadHash = ContentHash;
