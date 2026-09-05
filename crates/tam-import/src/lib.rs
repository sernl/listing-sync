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
use tam_domain::registry::{registry, NativeVocabulary};
use tam_marketplace::{ImportedListing, ListingState, RemoteListingId};
use tam_storage::{
    ElectionRepo, EventScope, JobRepo, MappingRepo, NewJob, OverrideRepo, ProductRepo, RaiseReport,
    RaiseScope, StorageError, TaxonomyRepo,
};
use tam_taxonomy::listing::{project_listing_with_overrides, ListingContext};
use tam_taxonomy::project::ingest_by_native_id;
use tam_taxonomy::TES_MAIN_AGE_RANGES;
use tam_types::{
    Actor, CanonicalTermId, ContentHash, CurrencyRule, FileBytes, FileId, FileKind, FileRole,
    ImportedPrice, ImportedTerm, InventoryId, JobEventPayload, JobId, ListingCopy, MappingId,
    Money, OrgId, PayloadSet, PriceIntent, ProductFile, ProductId, ScanOutcome, Stamp,
    SystemComponent, TermKind, Timestamp, Title, Uuid,
};

/// One file, as the caller has already decided it.
///
/// The kind travels beside the bytes because it is probed rather than
/// declared, and whoever probed it is the one who had the bytes.
pub struct ImportedFile {
    pub kind: FileKind,
    pub bytes: FileBytes,
}

/// One file whose bytes we hold, as the caller has already decided them.
///
/// The cover is one of these rather than an [`ImportedFile`], and that is a
/// guarantee rather than a convenience. Q-c has the device render the cover
/// itself and send it with the page, so a cover is always bytes we hold; a
/// marketplace-sourced cover would be an image the console cannot render,
/// pointing at a resource whose bytes are the seller's payload rather than a
/// thumbnail. Nothing downstream checks for it — `insert_file` writes what it
/// is handed — so the type refuses it instead, which is the same reason
/// `FileBytes` has two arms rather than nullable fields.
pub struct HeldFile {
    pub kind: FileKind,
    pub hash: ContentHash,
    pub byte_len: u64,
    pub scan: ScanOutcome,
}

/// One resource, ready to be applied.
///
/// Everything about where the bytes came from is already settled by the time
/// this exists, which is the point of the shape: this crate applies a listing
/// and no longer decides anything about files. The operator import ingests
/// from disk and hands over `Held` files; the device import hands over a
/// `Sourced` one it observed on the seller's own machine. Both are ordinary
/// here, and neither is a special case of the other.
pub struct AppliedResource {
    pub resource: i64,
    /// The listing as the source stated it, read by whoever held the session.
    pub listing: ImportedListing,
    /// At least one, because a product with nothing to sell cannot be listed
    /// and `PayloadSet` says so; an empty list is a named failure rather than
    /// a product with no payload.
    pub payload: Vec<ImportedFile>,
    pub cover: HeldFile,
}

/// Everything an import run holds constant across resources.
///
/// No adapter, no key and no object store: this crate reads no marketplace and
/// stores no bytes. It applies what a caller observed, which is what lets one
/// apply half serve both an operator importing from disk and a device
/// importing under the seller's own session.
pub struct ImportRun {
    pub pool: PgPool,
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
    /// Of the terms that mapped inbound, how many have nowhere declared to go
    /// in the target.
    ///
    /// The drain the kill gate reads, and it is deliberately the quantity
    /// `measure_one` has always computed rather than the listing projection's
    /// first blocker: Subject and Topic only, distinct terms, and a recorded
    /// no-counterpart excluded because that is a decision somebody took rather
    /// than a gap. Both callers get it from one helper so the definition
    /// cannot drift between them, which matters because the founder compares
    /// this number across a series of migrations.
    pub terms_uncovered: usize,
    pub unmapped_native_ids: Vec<String>,
    pub curriculum: Vec<String>,
    pub raised: RaiseReport,
    pub projectable: bool,
    pub blocked_by: Option<String>,
    /// How the source addresses the listing this row was read from, and which
    /// side of its draft line it sits on.
    ///
    /// A migrate's removal names both, and both come from the read that
    /// produced this row rather than from a second one. `state` is `None`
    /// where the read did not carry it, which is a refusal at the API and
    /// never a default: a removal must not post a lifecycle nobody observed.
    pub source: RemoteListingId,
    pub source_state: Option<ListingState>,
}

#[derive(Debug)]
pub enum ImportError {
    Storage(StorageError),
    NoPayload,
    Price(String),
    /// The request's stated intent has no lowering against a mapping the
    /// canonicalisation minted. Reachable rather than defensive: `live` on an
    /// inventory whose publish step is uncaptured refuses here.
    Lowering(tam_storage::LoweringRefusal),
    /// A paid listing on an inventory whose currency nobody has measured.
    /// Named apart from `Price` because it is not a malformed number: the
    /// source states an amount and renders a symbol, and reading that symbol
    /// as a currency would put an unmeasured denomination inside `Money`
    /// where nothing downstream can tell it from a measured one. The mirror
    /// of `ProjectionBlocked::CurrencyUnknown` on the read side, and removed
    /// by a probe rather than by a guess.
    CurrencyUnknown {
        inventory: InventoryId,
    },
}

impl core::fmt::Display for ImportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "storage: {error}"),
            Self::NoPayload => f.write_str("the entry carried no ingestable payload"),
            Self::Price(detail) => write!(f, "price: {detail}"),
            Self::Lowering(refusal) => write!(f, "lowering: {refusal}"),
            Self::CurrencyUnknown { inventory } => write!(
                f,
                "currency: {inventory:?} denominates prices per seller and none is measured"
            ),
        }
    }
}

impl core::error::Error for ImportError {}

impl From<StorageError> for ImportError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

/// One import run's drain measurement, folded from the per-row reports,
/// because the kill gate's unit is the catalogue rather than the row.
/// `terms_covered` is the canonical terms that already had a counterpart in
/// the target: the projection raises exactly one item per uncovered term, so
/// what the raise did not account for was covered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DrainTotals {
    pub rows: u64,
    pub terms_seen: u64,
    pub terms_unmapped: u64,
    pub terms_covered: u64,
    pub items_new: u64,
    pub items_already_open: u64,
}

impl DrainTotals {
    pub fn absorb(&mut self, report: &ImportRowReport) {
        let mapped = u64::try_from(report.terms_mapped).unwrap_or(u64::MAX);
        let raised = report.raised.new + report.raised.already_open;
        self.rows += 1;
        self.terms_seen += u64::try_from(report.terms_seen).unwrap_or(u64::MAX);
        self.terms_unmapped += u64::try_from(report.unmapped_native_ids.len()).unwrap_or(u64::MAX);
        self.terms_covered += mapped.saturating_sub(raised);
        self.items_new += report.raised.new;
        self.items_already_open += report.raised.already_open;
    }
}

fn wire(count: u64) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Opens the run's job row and records its drain measurement against it, so
/// the report the kill gate reads is a ledger row the client already knows
/// how to receive rather than a line on a terminal that scrolled away.
///
/// The job carries no items. An import writes nothing to a marketplace, and
/// a `queued` item is precisely what the worker's lease scan looks for, so
/// giving this job items would turn a measurement into a publish. Its
/// inventory is the source, the one the run's marketplace reads addressed;
/// the target travels in the payload, because the event stream carries the
/// body without its job row.
pub async fn record_drain_report(
    run: &ImportRun,
    totals: DrainTotals,
) -> Result<JobId, ImportError> {
    let job = JobId(fresh_uuid());
    let jobs = JobRepo::new(run.pool.clone());
    jobs.enqueue(
        run.org,
        &NewJob {
            job,
            inventory: run.source,
            stamp: Stamp {
                at: run.now,
                actor: Actor::System(SystemComponent::Import),
            },
        },
        &[],
        // The import command runs unattended against an export; nothing in
        // an ImportRun names a person, so it names itself instead.
    )
    .await?;
    jobs.record_event(
        &EventScope {
            org: run.org,
            job,
            item: None,
        },
        &JobEventPayload::ImportDrainMeasured {
            source: run.source,
            target: run.target,
            rows: wire(totals.rows),
            terms_seen: wire(totals.terms_seen),
            terms_unmapped: wire(totals.terms_unmapped),
            terms_covered: wire(totals.terms_covered),
            items_new: wire(totals.items_new),
            items_already_open: wire(totals.items_already_open),
        },
        Stamp::system(SystemComponent::Import, run.now),
    )
    .await?;
    Ok(job)
}

/// The catalogue coverage of one resource, measured without its files: how
/// many of the terms it carries reach a counterpart in the target inventory
/// and how many raise a reconciliation item. This is the kill-gate drain
/// contribution, computed through the same `project_terms` gate the full
/// import uses, so a measurement and a migration cannot disagree on coverage.
#[derive(Debug)]
pub struct MeasureReport {
    pub resource: i64,
    pub title: String,
    pub terms_seen: usize,
    pub terms_mapped: usize,
    pub unmapped_native_ids: Vec<String>,
    pub terms_uncovered: usize,
}

/// The catalogue's coverage, folded across resources. `share` is the drain
/// the kill gate compares across the first ten migrations: of the terms that
/// mapped inbound, the fraction with no counterpart into the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MeasureTotals {
    pub rows: u64,
    pub terms_seen: u64,
    pub terms_mapped: u64,
    pub terms_unmapped: u64,
    pub terms_uncovered: u64,
}

impl MeasureTotals {
    pub fn absorb(&mut self, report: &MeasureReport) {
        self.rows += 1;
        self.terms_seen += u64::try_from(report.terms_seen).unwrap_or(u64::MAX);
        self.terms_mapped += u64::try_from(report.terms_mapped).unwrap_or(u64::MAX);
        self.terms_unmapped += u64::try_from(report.unmapped_native_ids.len()).unwrap_or(u64::MAX);
        self.terms_uncovered += u64::try_from(report.terms_uncovered).unwrap_or(u64::MAX);
    }

    /// `None` when no term mapped, because a share over zero terms reads as
    /// perfect coverage when it is the absence of any signal at all.
    #[must_use]
    pub fn share(&self) -> Option<f64> {
        (self.terms_mapped != 0).then(|| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "term counts are small; the ratio is a display figure, not an accumulator"
            )]
            let share = self.terms_uncovered as f64 / self.terms_mapped as f64;
            share
        })
    }
}

/// Reads one resource and runs the taxonomy gate, without its files, its
/// product, or any persistence. The full import raises the reconciliation
/// items and settles a product; this only counts them.
pub async fn measure_one(
    run: &ImportRun,
    resource: i64,
    listing: &ImportedListing,
) -> Result<MeasureReport, ImportError> {
    let taxonomy = TaxonomyRepo::new(run.pool.clone());
    let (subjects, unmapped) = inbound_subjects(&taxonomy, run.source, listing).await?;
    let terms_seen = listing.native_ids(TermKind::Subject).len();
    let terms_mapped = subjects.len();

    let uncovered = uncovered_terms(&taxonomy, run.target, &subjects, &COVERAGE_AXES).await?;

    Ok(MeasureReport {
        resource,
        title: listing.title.clone(),
        terms_seen,
        terms_mapped,
        unmapped_native_ids: unmapped,
        terms_uncovered: uncovered.len(),
    })
}

/// The inbound projection shared by the full import and the measure path: the
/// seller's native category ids mapped to canonical subjects over the source
/// vocabulary's edges, an unmapped id retained verbatim.
async fn inbound_subjects(
    taxonomy: &TaxonomyRepo,
    source: InventoryId,
    listing: &tam_marketplace::ImportedListing,
) -> Result<(Vec<CanonicalTermId>, Vec<String>), ImportError> {
    // One slice over every vocabulary the source binds: `ingest_by_native_id`
    // filters by vocabulary itself, so two reads of the same relation are two
    // round trips for one answer.
    let edges = taxonomy
        .edges_into_all(&tam_taxonomy::routed_vocabularies(source))
        .await?;

    let mut subjects: Vec<CanonicalTermId> = Vec::new();
    let mut unmapped: Vec<String> = Vec::new();
    for native in &listing.native_ids(TermKind::Subject) {
        let found = ingest_by_native_id(
            native,
            tam_domain::VocabularyId(source, tam_domain::TermKind::Subject),
            &edges,
        )
        .or_else(|| {
            ingest_by_native_id(
                native,
                tam_domain::VocabularyId(source, tam_domain::TermKind::Topic),
                &edges,
            )
        });
        match found {
            Some(term) if !subjects.contains(&term) => subjects.push(term),
            Some(_) => {}
            None => unmapped.push(native.clone()),
        }
    }
    Ok((subjects, unmapped))
}

/// The axes the coverage number is measured over.
///
/// An explicit argument to [`uncovered_terms`] rather than an ambient constant,
/// and a named one rather than a literal at each call, because the founder
/// compares this number across a series of migrations: widening the routed
/// axes elsewhere in the tree must not silently redefine what the gate counts.
/// Changing this list is a deliberate change to the measurement.
pub const COVERAGE_AXES: [tam_domain::TermKind; 2] =
    [tam_domain::TermKind::Subject, tam_domain::TermKind::Topic];

/// Of the terms that mapped inbound, the ones with nowhere declared to go.
///
/// One definition, called by both the measurement and the import, because two
/// copies of a number somebody compares over time is how a definition drifts.
///
/// What counts as uncovered is narrower than "the projection refused", and the
/// difference is the whole value of the number. A term with a recorded
/// no-counterpart is omitted rather than blocked, because somebody decided that
/// axis does not cross and a decision is not a gap. A listing blocked for a
/// reason that is not a term at all — an unanswered election, an unmeasured
/// currency, a missing cover — contributes nothing here, because none of those
/// says anything about coverage. And a term the catalogue does not classify is
/// counted, matching the projection's fail-closed reading of an input it cannot
/// place.
pub async fn uncovered_terms(
    taxonomy: &TaxonomyRepo,
    target: InventoryId,
    subjects: &[CanonicalTermId],
    axes: &[tam_domain::TermKind],
) -> Result<Vec<CanonicalTermId>, ImportError> {
    let terms = taxonomy.terms().await?;
    let kinds: std::collections::HashMap<CanonicalTermId, tam_domain::TermKind> =
        terms.iter().map(|term| (term.id, term.kind)).collect();
    let target_edges = taxonomy
        .edges_into_all(&tam_taxonomy::routed_vocabularies(target))
        .await?;
    let no_counterparts = taxonomy.no_counterparts_into(target).await?;

    let mut uncovered: Vec<CanonicalTermId> = Vec::new();
    for kind in axes.iter().copied() {
        let of_kind: Vec<CanonicalTermId> = subjects
            .iter()
            .copied()
            .filter(|term| kinds.get(term) == Some(&kind))
            .collect();
        if of_kind.is_empty() {
            continue;
        }
        let outcome = tam_taxonomy::project::project_terms(
            &of_kind,
            tam_domain::VocabularyId(target, kind),
            &target_edges,
            &no_counterparts,
        );
        for blocked in outcome.blocked {
            if !uncovered.contains(&blocked.term) {
                uncovered.push(blocked.term);
            }
        }
    }
    for term in subjects {
        if !kinds.contains_key(term) && !uncovered.contains(term) {
            uncovered.push(*term);
        }
    }
    Ok(uncovered)
}

/// Applies one resource to the catalogue: a canonical product, its target
/// mapping, and one projection so every gap raises its queue item on the spot.
///
/// It reads no marketplace and stores no bytes. Everything about where the
/// files came from is settled before this is called, which is what lets one
/// apply half serve an operator importing from disk and a device importing
/// under the seller's own session.
pub async fn import_one(
    run: &ImportRun,
    applied: &AppliedResource,
) -> Result<ImportRowReport, ImportError> {
    let listing = applied.listing.clone();

    let payloads: Vec<ProductFile> = applied
        .payload
        .iter()
        .map(|file| ProductFile {
            id: FileId(fresh_uuid()),
            role: FileRole::Payload,
            kind: file.kind,
            bytes: file.bytes.clone(),
        })
        .collect();
    let cover = Some(ProductFile {
        id: FileId(fresh_uuid()),
        role: FileRole::Cover,
        kind: applied.cover.kind,
        bytes: FileBytes::Held {
            hash: applied.cover.hash,
            byte_len: applied.cover.byte_len,
            scan: applied.cover.scan.clone(),
        },
    });
    let mut payload_iter = payloads.into_iter();
    let head = payload_iter.next().ok_or(ImportError::NoPayload)?;
    let payload = Some(PayloadSet::new(head, payload_iter.collect()));

    // Taxonomy inbound by native id over the source vocabulary's edges. An
    // unmapped id is retained verbatim in the report — the item type cannot
    // name a canonical term that does not exist — and the outbound raise
    // below covers every term that does.
    let taxonomy = TaxonomyRepo::new(run.pool.clone());
    let (subjects, unmapped) = inbound_subjects(&taxonomy, run.source, &listing).await?;
    // The coverage number, from the one helper the measurement also uses, and
    // taken here while the mapped terms are still in hand rather than after
    // the product has consumed them. It runs on those terms rather than on the
    // projection's outcome, so a listing blocked on a currency or a cover still
    // reports a truthful count instead of a zero produced by never reaching
    // the taxonomy.
    let uncovered = uncovered_terms(&taxonomy, run.target, &subjects, &COVERAGE_AXES).await?;
    let terms_seen = listing.native_ids(TermKind::Subject).len();
    let terms_mapped = subjects.len();

    // Grades verbatim: the declared age-range ids as paths, the label looked
    // up from the measured table where it exists, the interval derived only
    // when every declared range is bounded.
    let grade_paths: Vec<tam_domain::VocabularyPath> = listing
        .native_ids(TermKind::Phase)
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

    let price = resolve_price(run.source, &listing.price)?;
    let rights = rights_from(&listing);
    let native_residue = residue_of(&listing);

    let product_id = ProductId(fresh_uuid());
    let product = tam_domain::CanonicalProduct {
        id: product_id,
        org: run.org,
        title: Title(listing.title.clone()),
        body: ListingCopy {
            // The adapter that read the body declares its format; sniffing it
            // back out of the bytes is exactly what the declaration exists to
            // avoid, and hardcoding one stores every TPT product -- whose
            // description is HTML -- as markdown.
            body: listing.body.clone(),
            format: listing.body_format,
        },
        payload,
        cover,
        previews: vec![],
        subjects,
        grades,
        price,
        rights,
        native_residue,
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
    let target_edges = taxonomy
        .edges_into_all(&tam_taxonomy::projection_vocabularies(run.target, &product))
        .await?;
    let no_counterparts = taxonomy.no_counterparts_into(run.target).await?;
    let elections = ElectionRepo::new(run.pool.clone());
    let rules = elections.rules(run.org).await?;
    // A freshly minted product has settled nothing, and the read is here
    // anyway so the two projection sites stay one shape rather than two.
    let settled = elections.answered_for(run.org, product_id).await?;
    // This projection is outbound — the imported product into `run.target` —
    // so the seller's own mapping decisions apply to it exactly as they apply
    // to a sync run's. Reading them here is what makes the import's gap report
    // the same measurement the engine would produce, rather than one that
    // raises gaps the seller has already answered.
    let overrides = OverrideRepo::new(run.pool.clone()).for_org(run.org).await?;
    let outcome = project_listing_with_overrides(
        &product,
        &ListingContext {
            org: run.org,
            mapping: mapping_id,
            inventory: run.target,
            now: run.now,
            terms: &terms,
            edges: &target_edges,
            no_counterparts: &no_counterparts,
            rules: &rules,
            settled: &settled,
        },
        &overrides,
    );
    let (projectable, blocked_by, raised) = match outcome {
        Ok(projection) => {
            record_losses(run, mapping_id, &projection.loss).await?;
            (
                true,
                None,
                RaiseReport {
                    new: 0,
                    already_open: 0,
                },
            )
        }
        Err(tam_domain::ProjectionBlocked::Blocked {
            gaps,
            elections,
            loss,
            ..
        }) => {
            record_losses(run, mapping_id, &loss).await?;
            let causes: Vec<(CanonicalTermId, tam_domain::TermKind)> = gaps
                .iter()
                .map(|gap| {
                    let tam_domain::VocabularyId(_, kind) = gap.target;
                    (gap.term, kind)
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
            let gate = if gaps.is_empty() && !elections.is_empty() {
                "election"
            } else {
                "taxonomy"
            };
            (false, Some(gate.to_owned()), raised)
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
        resource: applied.resource,
        product: product_id,
        mapping: mapping_id,
        title: listing.title.clone(),
        terms_seen,
        terms_mapped,
        terms_uncovered: uncovered.len(),
        unmapped_native_ids: unmapped,
        curriculum: curriculum_of(&listing),
        raised,
        projectable,
        blocked_by,
        source: listing.remote.clone(),
        source_state: listing.state,
    })
}

/// The create job's items for one request: the seller's stated intent lowered
/// against each mapping the import minted, each with its idempotency key.
///
/// Here rather than in `tam-sync-worker` because there are now two callers and
/// the rows they mint must be identical: the cron drain, and the device
/// import's completing page. It builds the items and mints nothing, which is
/// what lets the second caller write them inside the same transaction that
/// settles its request — `tam-sync-worker` hands them to
/// `JobRepo::create_with_request_key`, `tam-api` hands them to
/// `SyncRequestRepo::complete_with_create_job`, and neither has a copy of this
/// lowering.
pub async fn create_items(
    run: &ImportRun,
    intent: tam_storage::SyncIntent,
    job: JobId,
    mappings: &[MappingId],
) -> Result<Vec<tam_storage::NewJobItem>, ImportError> {
    let seeds = tam_storage::JobReadRepo::new(run.pool.clone())
        .mapping_seeds(run.org, run.target, mappings)
        .await?;
    let to = match intent {
        tam_storage::SyncIntent::Draft => ListingState::Draft,
        tam_storage::SyncIntent::Live => ListingState::Live,
    };
    let mut items: Vec<tam_storage::NewJobItem> = Vec::new();
    for seed in &seeds {
        for operation in tam_storage::lower(to, run.target, seed).map_err(ImportError::Lowering)? {
            items.push(tam_storage::NewJobItem {
                item: tam_domain::JobItemId(fresh_uuid()),
                mapping: seed.mapping,
                idempotency_key: tam_marketplace::idempotency::derive_idempotency_key(
                    run.org,
                    run.target,
                    seed.product,
                    INTENT_VERSION,
                    tam_storage::job_reads::intent_digest(
                        &operation,
                        job,
                        &seed.payload_hashes,
                        seed.sever_generation,
                    ),
                ),
                requires_bound_on: tam_storage::requires_bound_on(&operation, run.target),
                operation,
            });
        }
    }
    Ok(items)
}

/// The intent version the idempotency key is derived under. One constant for
/// both callers, because two copies of it would make the cron drain and the
/// device import derive different keys for the same write.
pub const INTENT_VERSION: u32 = 1;

/// The price the source stated, denominated where the inventory's own rule/// The price the source stated, denominated where the inventory's own rule
/// states the currency and blocked where it does not.
///
/// This is not the licence decoder it replaces. Which licences are paid is
/// the source marketplace's own fact and belongs in its adapter, which is
/// where the seven-row table now lives; what remains here is resolving a
/// stated amount against the inventory's currency rule.
///
/// A seller-scoped inventory renders a bare symbol, and reading that as USD
/// would put an unmeasured currency inside `Money`, where it is
/// indistinguishable from a measured one and the parity and price-floor
/// guards compare across the wrong denomination. `CurrencyUnknown` is the
/// designed answer and the variant's own doc says a probe removes it, not a
/// guess -- which is what happened to TPT, whose rule the founder settled in
/// the seller account rather than by reading its dollar sign.
fn resolve_price(source: InventoryId, price: &ImportedPrice) -> Result<PriceIntent, ImportError> {
    let ImportedPrice::Paid {
        minor_units,
        denomination,
    } = price
    else {
        return Ok(PriceIntent::Free);
    };
    let CurrencyRule::Fixed(currency) = source.currency_rule() else {
        return Err(ImportError::CurrencyUnknown { inventory: source });
    };
    if currency.code() != denomination {
        return Err(ImportError::Price(format!(
            "{source:?} fixes {} and the source read {denomination:?}",
            currency.code()
        )));
    }
    Money::new(*minor_units, currency)
        .map(PriceIntent::Paid)
        .map_err(|error| ImportError::Price(error.to_string()))
}

/// D4: the grant the source stated, kept as the source's own value. Read and
/// discarded before this existed, which is why every product before it reads
/// back `Unstated`.
fn rights_from(listing: &tam_marketplace::ImportedListing) -> tam_domain::RightsDeclaration {
    listing
        .rights
        .as_ref()
        .map_or(tam_domain::RightsDeclaration::Unstated, |term| {
            tam_domain::RightsDeclaration::Declared {
                source: tam_domain::VocabularyPath {
                    vocabulary: tam_domain::VocabularyId(term.inventory, TermKind::Licence),
                    segments: term.segments.clone(),
                    native_id: term.native_id.clone(),
                },
            }
        })
}

/// Every source value in an axis this model does not type, kept verbatim so a
/// round trip back to the source loses nothing and a projection into a
/// platform without the field can name what it dropped.
fn residue_of(listing: &tam_marketplace::ImportedListing) -> Vec<ImportedTerm> {
    listing
        .native
        .iter()
        .filter(|term| term.kind.is_none())
        .cloned()
        .collect()
}

/// The operator-facing curriculum column, derived from the residue rather
/// than carried as its own field. `curriculum` is a read-only native field
/// with a twelve-value closed vocabulary and no axis of its own, so it
/// arrives untagged and is recognised by its membership.
fn curriculum_of(listing: &tam_marketplace::ImportedListing) -> Vec<String> {
    let Some(NativeVocabulary::Closed(known)) = registry(listing_inventory(listing))
        .native("curriculum")
        .map(|field| field.vocabulary)
    else {
        return Vec::new();
    };
    listing
        .native
        .iter()
        .filter(|term| term.kind.is_none())
        .filter_map(|term| term.native_id.clone())
        .filter(|value| known.contains(&value.as_str()))
        .collect()
}

/// Which inventory's registry describes the values a listing carried. Every
/// term a read yields names its own inventory, and a listing that carried
/// none has no residue to classify.
fn listing_inventory(listing: &tam_marketplace::ImportedListing) -> InventoryId {
    listing
        .native
        .first()
        .map_or(InventoryId::TesGb, |term| term.inventory)
}

/// What the import's own projection could not carry, recorded against the
/// mapping it just minted.
///
/// The import projects once immediately, which is what makes its report the
/// drain measurement; the losses that projection measured belong to the same
/// record, so a Tes-to-TPT licence drop is visible from the moment the product
/// exists rather than only after a sync run has leased it.
async fn record_losses(
    run: &ImportRun,
    mapping: MappingId,
    losses: &[tam_domain::equivalence::Loss],
) -> Result<(), ImportError> {
    if losses.is_empty() {
        return Ok(());
    }
    MappingRepo::new(run.pool.clone())
        .record_losses(
            tam_storage::LossScope {
                org: run.org,
                mapping,
                attempt: tam_types::AttemptId(fresh_uuid()),
                at: run.now,
            },
            losses,
        )
        .await?;
    Ok(())
}

/// Row identity, minted at the import boundary.
fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::{resolve_price, ImportError};
    use tam_types::{Currency, ImportedPrice, InventoryId, Money, PriceIntent};

    fn paid(minor_units: i64, denomination: &str) -> ImportedPrice {
        ImportedPrice::Paid {
            minor_units,
            denomination: denomination.to_owned(),
        }
    }

    #[test]
    fn a_fixed_inventory_denominates_a_price_from_its_own_rule() {
        assert_eq!(
            resolve_price(InventoryId::TesGb, &paid(450, "GBP")).ok(),
            Money::new(450, Currency::Gbp).ok().map(PriceIntent::Paid),
            "the write side already mints from currency_rule, and the read side agrees"
        );
        assert_eq!(
            resolve_price(InventoryId::TesUs, &paid(450, "USD")).ok(),
            Money::new(450, Currency::Usd).ok().map(PriceIntent::Paid),
            "TesUs fixes USD; a rule that said Tes mints GBP would be wrong here"
        );
    }

    #[test]
    fn a_seller_scoped_inventory_refuses_rather_than_reading_a_symbol_as_a_currency() {
        // A symbol-to-currency rule would put an unmeasured currency inside
        // Money, where it is indistinguishable from a measured one and the
        // parity and price-floor guards then compare across the wrong
        // denomination.
        let refused = resolve_price(InventoryId::Etsy, &paid(495, "$"));
        assert!(
            matches!(refused, Err(ImportError::CurrencyUnknown { inventory }) if inventory == InventoryId::Etsy),
            "a seller-scoped currency is unmeasured, and the variant's own doc says a probe \
             removes it rather than a guess"
        );
    }

    /// The probe that removed the variant for TPT: the founder confirmed in
    /// their own seller account on 2026-08-29 that the marketplace sells in
    /// USD and offers nothing else.
    #[test]
    fn the_tpt_inventory_denominates_from_the_currency_the_founder_confirmed() {
        assert_eq!(
            resolve_price(InventoryId::Tpt, &paid(495, "USD")).ok(),
            Money::new(495, Currency::Usd).ok().map(PriceIntent::Paid),
            "TPT sells in one currency, so the price is denominated rather than blocked"
        );
        assert!(
            matches!(
                resolve_price(InventoryId::Tpt, &paid(495, "GBP")),
                Err(ImportError::Price(_))
            ),
            "and a pound amount is refused rather than redenominated, because the wire \
             carries a bare number and would sell it as that many dollars"
        );
    }

    #[test]
    fn a_free_listing_needs_no_currency_at_all() {
        assert_eq!(
            resolve_price(InventoryId::Tpt, &ImportedPrice::Free).ok(),
            Some(PriceIntent::Free),
            "the currency gate applies to a price, and a free listing has none"
        );
    }

    #[test]
    fn a_denomination_the_inventory_does_not_fix_is_refused() {
        assert!(
            matches!(
                resolve_price(InventoryId::TesGb, &paid(450, "USD")),
                Err(ImportError::Price(_))
            ),
            "reading a USD price into a GBP-fixed inventory would silently redenominate it"
        );
    }
}
