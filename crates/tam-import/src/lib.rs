//! The catalogue import: the seller's own marketplace listing, read under
//! the first-party-export capability, canonicalised into a product with its
//! files ingested, its taxonomy mapped inbound over the seeded crosswalk,
//! its grade declaration retained verbatim, and its target mapping created —
//! then projected once, so every gap raises its queue item on the spot and
//! the drain report the kill gate reads falls out of the run itself.
//!
//! Customer zero's file bytes arrive from disk (the founder has the
//! originals). A catalogue read hands over no file of its own: the bytes
//! come from the seller's own download hop, which a migration's device
//! performs under the seller's session, so this crate applies what its
//! caller observed and fetches nothing.

#![forbid(unsafe_code)]

use sqlx::PgPool;
use tam_domain::registry::{registry, NativeVocabulary};
use tam_marketplace::{ImportedListing, ListingState, RemoteListingId};
use tam_storage::{
    bind_listing, insert_mapping, insert_product, raise_taxonomy_gaps, record_mapping_losses,
    ElectionRepo, EventScope, JobRepo, NewJob, OverrideRepo, RaiseReport, RaiseScope, StorageError,
    TaxonomyRepo,
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
    /// The identifier the product will have.
    ///
    /// The caller's rather than minted here, which is the same convention the
    /// spreadsheet commit already follows: a row reserves its product
    /// identifier before anything is created, so a pass killed mid-create is
    /// resumed by reading whether that product exists rather than by minting a
    /// second one. An import run's items reserve theirs for a second reason as
    /// well -- the matcher needs a stable name for a side of a pair before
    /// either side is a product.
    pub product: ProductId,
    /// The listing as the source stated it, read by whoever held the session.
    pub listing: ImportedListing,
    /// Empty only where the source could not hand over a file at all.
    ///
    /// D32 moved the payload requirement from the product to the mapping
    /// (migration 0061), so a resource kept on Teachouse alone may carry
    /// nothing: a catalogue-only import is exactly that case, because the
    /// read describes a listing and names no file, whichever marketplace it
    /// came from. Such an import names no target and therefore no mapping,
    /// so the trigger that would refuse it does not fire.
    pub payload: Vec<ImportedFile>,
    /// The cover, where one could be rendered. Absent exactly when the
    /// payload is: the cover is derived from the file, so no file is no cover.
    pub cover: Option<HeldFile>,
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
    /// Where the imported resource is to be drafted, or `None` for an import
    /// that drafts nowhere.
    ///
    /// A catalogue-only import is the ordinary case since phase 2: the outcome
    /// is resources in the catalogue with nothing drafted anywhere, so there
    /// is no target mapping to mint, no outbound projection to run and no
    /// coverage to measure. An `Option` rather than a sentinel inventory,
    /// because reading a target that means "nowhere" is exactly the trap
    /// migration 0053 names.
    pub target: Option<InventoryId>,
    /// The copy/move confirmation whose frozen policy precedes this source read.
    pub request: Option<Uuid>,
    pub now: Timestamp,
}

/// What one entry became, and what the drain learned from it.
#[derive(Debug)]
pub struct ImportRowReport {
    pub resource: i64,
    pub product: ProductId,
    /// The target mapping this import minted, or `None` where the run named no
    /// target and so minted none.
    pub mapping: Option<MappingId>,
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
    /// A call that only means something against a target inventory was made
    /// on a run that names none.
    ///
    /// Named rather than defaulted, because every alternative is worse: a
    /// sentinel inventory would draft into a shop nobody chose, and silently
    /// answering "nothing" would report a drain of zero for a measurement that
    /// never ran. The drain report, the coverage measurement and the create
    /// job all belong to the migration path, which always names a target.
    NoTarget,
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
            Self::NoTarget => f.write_str(
                "this import names no target inventory, so there is nothing to measure or draft                  against",
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

/// The target a call needs, or the named refusal.
///
/// One helper rather than an `unwrap_or` per site, so the three calls that
/// only mean something against a target — the drain report, the coverage
/// measurement and the create job — answer identically and none of them can
/// acquire a default by accident.
fn target_of(run: &ImportRun) -> Result<InventoryId, ImportError> {
    run.target.ok_or(ImportError::NoTarget)
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
            target: target_of(run)?,
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
    let inbound = inbound_terms(&taxonomy, run.source, listing).await?;
    let terms_seen = inbound.subjects.len() + inbound.unmapped.len();
    let terms_mapped = inbound.subjects.len();

    let uncovered = uncovered_terms(
        &taxonomy,
        target_of(run)?,
        &inbound.subjects,
        &COVERAGE_AXES,
    )
    .await?;

    Ok(MeasureReport {
        resource,
        title: listing.title.clone(),
        terms_seen,
        terms_mapped,
        unmapped_native_ids: inbound.unmapped,
        terms_uncovered: uncovered.len(),
    })
}

/// What the source's own values map to over its vocabulary's edges.
///
/// The subject axis and the resource type are held apart because the
/// coverage counters are a measurement over subjects and topics alone, taken
/// before the resource type was read at all; folding the type into them would
/// move a number somebody compares over time.
struct InboundTerms {
    subjects: Vec<CanonicalTermId>,
    /// A native subject or topic id the relation does not know, verbatim.
    unmapped: Vec<String>,
    resource_type: Option<CanonicalTermId>,
    /// The native ids the relation places on the source's phase axis, in
    /// listing order: the grade declaration reads these rather than the
    /// tagged axis alone.
    phases: Vec<String>,
}

impl InboundTerms {
    /// Every canonical term the product carries, in the order the projection
    /// filters them by axis.
    fn all(&self) -> Vec<CanonicalTermId> {
        let mut terms = self.subjects.clone();
        terms.extend(self.resource_type);
        terms
    }
}

/// The native ids on one axis: the ones the reader tagged with it, and the
/// untagged ones the source's relation binds to it.
///
/// TPT's reader tags nothing, deliberately -- its facets are one flat
/// namespace, and which axis a slug answers is a fact of the seeded relation
/// rather than of the array it came out of. So the relation is consulted
/// here: an untagged id with an exact edge into the axis's vocabulary is on
/// that axis. Before this the untagged ids were never asked about and every
/// TPT import landed with no subject, no type and no grade, which Tes's
/// publish then refused (2026-09-19).
fn native_ids_on(
    listing: &tam_marketplace::ImportedListing,
    source: InventoryId,
    kind: tam_domain::TermKind,
    edges: &[tam_domain::ProjectionEdge],
) -> Vec<String> {
    let vocabulary = tam_domain::VocabularyId(source, kind);
    let mut ids: Vec<String> = Vec::new();
    for term in &listing.native {
        let Some(native) = term.native_id.as_deref() else {
            continue;
        };
        let on_axis = match term.kind {
            Some(tagged) => tagged == kind,
            None => ingest_by_native_id(native, vocabulary, edges).is_some(),
        };
        if on_axis && !ids.iter().any(|id| id == native) {
            ids.push(native.to_owned());
        }
    }
    ids
}

/// The inbound projection shared by the full import and the measure path: the
/// seller's native ids mapped to canonical terms over the source vocabulary's
/// edges, an unmapped subject retained verbatim, and the resource type and
/// phases read over the same relation.
async fn inbound_terms(
    taxonomy: &TaxonomyRepo,
    source: InventoryId,
    listing: &tam_marketplace::ImportedListing,
) -> Result<InboundTerms, ImportError> {
    // One slice over every vocabulary the source binds: `ingest_by_native_id`
    // filters by vocabulary itself, so two reads of the same relation are two
    // round trips for one answer.
    let edges = taxonomy
        .edges_into_all(&tam_taxonomy::routed_vocabularies(source))
        .await?;

    let mut subjects: Vec<CanonicalTermId> = Vec::new();
    let mut unmapped: Vec<String> = Vec::new();
    let mut on_subject_axes = native_ids_on(listing, source, tam_domain::TermKind::Subject, &edges);
    for topic in native_ids_on(listing, source, tam_domain::TermKind::Topic, &edges) {
        if !on_subject_axes.contains(&topic) {
            on_subject_axes.push(topic);
        }
    }
    for native in &on_subject_axes {
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
    let resource_type = native_ids_on(listing, source, tam_domain::TermKind::ResourceType, &edges)
        .iter()
        .find_map(|native| {
            ingest_by_native_id(
                native,
                tam_domain::VocabularyId(source, tam_domain::TermKind::ResourceType),
                &edges,
            )
        });
    let phases = native_ids_on(listing, source, tam_domain::TermKind::Phase, &edges);
    Ok(InboundTerms {
        subjects,
        unmapped,
        resource_type,
        phases,
    })
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

/// One resource, prepared: everything the catalogue write needs, with every
/// read that does not have to happen inside the decision transaction already
/// done.
///
/// The halves exist because of what the import's commit has to be able to
/// promise. The product, the listing it was read from and the import row's
/// outcome are one decision, so they are written in one transaction under one
/// organisation-scoped lock — and nothing slow may happen under that lock.
/// So the taxonomy reads, the price resolution and the grade derivation
/// happen here, before it is taken.
pub struct PreparedResource {
    /// The row report's own handle on the resource.
    pub resource: i64,
    pub product: tam_domain::CanonicalProduct,
    /// The listing this was read from, as a binding on the source.
    pub source_mapping: tam_domain::Mapping,
    pub listing: ImportedListing,
    pub terms_seen: usize,
    pub terms_mapped: usize,
    pub terms_uncovered: usize,
    pub unmapped_native_ids: Vec<String>,
    /// The price the source stated, resolved once and reused by the target
    /// mapping.
    pub price: PriceIntent,
}

impl PreparedResource {
    /// What this resource became for a run that drafts nowhere.
    ///
    /// No mapping, neither projectable nor blocked and nothing raised: those
    /// are answers about a destination, and a catalogue-only import has none.
    #[must_use]
    pub fn catalogue_report(&self) -> ImportRowReport {
        ImportRowReport {
            resource: self.resource,
            product: self.product.id,
            mapping: None,
            title: self.listing.title.clone(),
            terms_seen: self.terms_seen,
            terms_mapped: self.terms_mapped,
            terms_uncovered: self.terms_uncovered,
            unmapped_native_ids: self.unmapped_native_ids.clone(),
            curriculum: curriculum_of(&self.listing),
            raised: RaiseReport {
                new: 0,
                already_open: 0,
            },
            projectable: false,
            blocked_by: None,
            source: self.listing.remote.clone(),
            source_state: self.listing.state,
        }
    }
}

/// Reads everything one resource's catalogue write depends on, and writes
/// nothing.
///
/// # Errors
///
/// A price that will not denominate, a currency nobody has measured, or a
/// resource with no payload where the run drafts onto a marketplace: each is
/// something about the seller's own listing rather than a fault.
pub async fn prepare_one(
    run: &ImportRun,
    applied: &AppliedResource,
) -> Result<PreparedResource, ImportError> {
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
    let cover = applied.cover.as_ref().map(|cover| ProductFile {
        id: FileId(fresh_uuid()),
        role: FileRole::Cover,
        kind: cover.kind,
        bytes: FileBytes::Held {
            hash: cover.hash,
            byte_len: cover.byte_len,
            scan: cover.scan.clone(),
        },
    });
    // A run with a target drafts onto a marketplace and so must carry a file;
    // a catalogue-only run mints no target mapping, and D32 says a resource
    // kept here alone may carry none. So the refusal follows the target rather
    // than the list: reading an empty payload as a fault would refuse every
    // catalogue-only import, whose read names no file.
    let mut payload_iter = payloads.into_iter();
    let payload = match payload_iter.next() {
        Some(head) => Some(PayloadSet::new(head, payload_iter.collect())),
        None if run.target.is_none() => None,
        None => return Err(ImportError::NoPayload),
    };

    // Taxonomy inbound by native id over the source vocabulary's edges. An
    // unmapped id is retained verbatim in the report — the item type cannot
    // name a canonical term that does not exist — and the outbound raise
    // covers every term that does.
    let taxonomy = TaxonomyRepo::new(run.pool.clone());
    let inbound = inbound_terms(&taxonomy, run.source, &listing).await?;
    // The coverage number, from the one helper the measurement also uses, and
    // taken here while the mapped terms are still in hand rather than after
    // the product has consumed them. Only against a target: "how many of this
    // resource's terms have nowhere to go" is a question about a destination,
    // and a run with none has not measured zero of them.
    let uncovered = match run.target {
        Some(target) => uncovered_terms(&taxonomy, target, &inbound.subjects, &COVERAGE_AXES)
            .await?
            .len(),
        None => 0,
    };
    let terms_seen = inbound.subjects.len() + inbound.unmapped.len();
    let terms_mapped = inbound.subjects.len();

    // Grades verbatim: the declared age-range ids as paths, the label looked
    // up from the measured table where it exists, the interval derived only
    // when every declared range is bounded.
    let grade_paths: Vec<tam_domain::VocabularyPath> = inbound
        .phases
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

    let product_id = applied.product;
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
        subjects: inbound.all(),
        grades,
        price,
        rights,
        native_residue,
    };

    let source_mapping = source_binding(run.org, product_id, run.source, &listing, price, run.now);

    Ok(PreparedResource {
        resource: applied.resource,
        product,
        source_mapping,
        listing,
        terms_seen,
        terms_mapped,
        terms_uncovered: uncovered,
        unmapped_native_ids: inbound.unmapped,
        price,
    })
}

/// The mapping that records the listing a read came from.
///
/// Public and shared, because two paths need exactly this record: the commit
/// that creates the product, and the commit that decides the resource is one
/// the catalogue already holds and binds this shop's listing onto the
/// survivor instead. A second construction of it would be a second answer to
/// "where is this resource listed".
///
/// A record of what exists rather than a draft: nothing is projected, nothing
/// is enqueued, and the mode is `DryRun`, so no write to the source can
/// follow from it. A read that did not carry the listing's state — every TPT
/// capture on file — binds with `Absent`, the one lifecycle that claims
/// nothing about the listing's side of the draft line; a Copy still works
/// from it and a Move is refused until the state is verified.
#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "the tenant, the product, the shop, the listing, its resolved price and the instant; every one comes from a different place and both callers pass all six"
)]
pub fn source_binding(
    org: OrgId,
    product: ProductId,
    source: InventoryId,
    listing: &ImportedListing,
    price: PriceIntent,
    now: Timestamp,
) -> tam_domain::Mapping {
    tam_domain::Mapping {
        id: MappingId(fresh_uuid()),
        org,
        product,
        inventory: source,
        binding: tam_domain::Binding::Bound {
            id: listing.remote.clone(),
            first_seen: now,
            verified: tam_domain::Verification::Clean { at: now },
        },
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
        lifecycle: match listing.state {
            Some(ListingState::Live) => tam_marketplace::RemoteLifecycle::Live { since: now },
            Some(ListingState::Draft) => tam_marketplace::RemoteLifecycle::Draft,
            None => tam_marketplace::RemoteLifecycle::Absent,
        },
    }
}

/// Writes the product and the listing it was read from, in a transaction the
/// caller owns.
///
/// The catalogue half of an import and the whole of it for a run that drafts
/// nowhere.
///
/// A listing another product of this organisation already binds is left where
/// it is rather than failing the import: that is the duplicate review's
/// question, not this write's, and the resource still lands. The check is a
/// read before the write rather than a caught unique violation, because
/// PostgreSQL aborts the whole transaction on the violation — catching the
/// Rust error inside a caller's transaction would roll back the product this
/// call had just inserted while the caller carried on as though it existed.
///
/// The binding is written only where the product carries a payload, and that
/// is not an optimisation. Migration 0061 moved the file requirement from the
/// product to the mapping: a product no marketplace carries may have no file
/// yet, and a mapping onto a payload-less product raises
/// `product % reaches a marketplace and has no live payload file` at commit.
/// Every catalogue read that named no file is exactly that product, so
/// binding it unconditionally rolled the whole commit back and the item was
/// retried forever. The metadata and the product land; the listing binds when
/// the file arrives, which is the download hop's own step.
///
/// Answers whether the source binding was written.
///
/// # Errors
///
/// Only storage: everything a seller can act on was decided in
/// [`prepare_one`].
pub async fn apply_prepared(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    prepared: &PreparedResource,
    now: Timestamp,
) -> Result<bool, ImportError> {
    insert_product(
        tx,
        org,
        &prepared.product,
        &std::collections::HashMap::new(),
        now,
    )
    .await?;
    if prepared.product.payload.is_none() {
        return Ok(false);
    }
    bind_listing(tx, org, &prepared.source_mapping, 0, now).await?;
    Ok(true)
}

/// One resource, prepared whole: the catalogue half and, where the run names
/// a target, the mapping it will mint together with the projection that
/// mapping already answered.
///
/// Opaque, because the only thing a caller does with it is apply it. Its
/// point is the split it forces: every read this import depends on — the
/// taxonomy terms, both sides' edges, the recorded no-counterparts, the
/// election rules and answers, the seller's overrides — and the whole of the
/// pure projection happen while it is built, so the transaction that applies
/// it touches nothing but that transaction. A caller may therefore hold the
/// product, its bindings, the target mapping and the projection's own records
/// inside its own decision — an API page's receipt, say — and roll every one
/// of them back together.
pub struct PreparedImport {
    resource: PreparedResource,
    /// `None` for a run that drafts nowhere, which is the ordinary catalogue
    /// import: no mapping to mint, no projection to run.
    target: Option<PreparedTarget>,
}

/// The target half of a prepared import: the mapping to be written and what
/// the projection answered about it.
///
/// The outcome is the projection's own return type rather than a summary of
/// it. Everything the application writes — the losses either arm measured,
/// the gaps a block raises — is already in there, and restating it would put
/// the gate's vocabulary in a second place.
struct PreparedTarget {
    mapping: tam_domain::Mapping,
    outcome: Result<tam_domain::ListingProjection, tam_domain::ProjectionBlocked>,
}

/// Reads everything one import depends on, pure-projects it, and writes
/// nothing.
///
/// # Errors
///
/// Whatever [`prepare_one`] refuses, plus storage for the projection's own
/// reads.
pub async fn prepare_import(
    run: &ImportRun,
    applied: &AppliedResource,
) -> Result<PreparedImport, ImportError> {
    let resource = prepare_one(run, applied).await?;
    let target = match run.target {
        Some(inventory) => Some(prepare_target(run, &resource, inventory).await?),
        None => None,
    };
    Ok(PreparedImport { resource, target })
}

/// Writes one prepared import through a transaction the caller owns: the
/// product, its source binding, the target mapping, the projection's losses
/// and the reconciliation items its gaps raise.
///
/// Every write goes through `tx` and nothing else is touched — no pool read,
/// no second transaction, no commit. That is the whole contract: the caller
/// decides when this becomes true, and may bind its own record to the same
/// decision. The tenant must already be pinned on `tx`.
///
/// # Errors
///
/// Only storage: everything a seller can act on was decided in
/// [`prepare_import`].
pub async fn apply_import(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    prepared: &PreparedImport,
    now: Timestamp,
) -> Result<ImportRowReport, ImportError> {
    // Whether the source listing bound is not this caller's business: a read
    // whose bytes are uncaptured lands as a product without one, and the
    // report's catalogue half says the same either way.
    let _bound = apply_prepared(tx, org, &prepared.resource, now).await?;
    let mut report = prepared.resource.catalogue_report();
    let Some(target) = prepared.target.as_ref() else {
        return Ok(report);
    };
    insert_mapping(tx, org, &target.mapping, 0, now).await?;
    let projected = apply_projection(tx, org, target, now).await?;
    report.mapping = Some(target.mapping.id);
    report.projectable = projected.projectable;
    report.blocked_by = projected.blocked_by;
    report.raised = projected.raised;
    Ok(report)
}

/// Applies one resource to the catalogue: a canonical product, its target
/// mapping, and one projection so every gap raises its queue item on the spot.
///
/// It reads no marketplace and stores no bytes. Everything about where the
/// files came from is settled before this is called, which is what lets one
/// apply half serve an operator importing from disk and a device importing
/// under the seller's own session.
///
/// The convenience for a caller with no record of its own to bind: prepare,
/// then one tenant-pinned transaction around the whole application. The
/// product, its source binding, the target mapping and the projection's
/// records land together or not at all, so a run killed mid-import leaves no
/// half-imported resource for a replay to duplicate.
pub async fn import_one(
    run: &ImportRun,
    applied: &AppliedResource,
) -> Result<ImportRowReport, ImportError> {
    let prepared = prepare_import(run, applied).await?;
    let mut tx = run.pool.begin().await.map_err(StorageError::from)?;
    tam_storage::pin_tenant(&mut tx, run.org).await?;
    let report = apply_import(&mut tx, run.org, &prepared, run.now).await?;
    tx.commit().await.map_err(StorageError::from)?;
    Ok(report)
}

/// What the target mapping's one projection answered.
struct Projected {
    projectable: bool,
    blocked_by: Option<String>,
    raised: RaiseReport,
}

/// The target mapping and the one projection, both of which exist only for a
/// run that names a target.
///
/// A catalogue-only import is the ordinary case since phase 2: the outcome is
/// a resource in the catalogue, bound to where it already is and drafted
/// nowhere, so there is no target mapping to mint, no gaps to raise and no
/// coverage to report. Skipping them is not a degraded import -- it is the
/// whole of what "nothing drafted" means.
///
/// Every read is here rather than beside the write. The projection is pure
/// once its inputs are in hand, and its inputs are the same whether the
/// mapping row exists yet or not: the mapping is an argument to the
/// projection, not a row it consults.
async fn prepare_target(
    run: &ImportRun,
    prepared: &PreparedResource,
    target: InventoryId,
) -> Result<PreparedTarget, ImportError> {
    let product = &prepared.product;
    let output = match run.request {
        Some(request) => {
            tam_storage::rule_capture::confirmed_output(&run.pool, run.org, request, product)
                .await?
        }
        None => Some(
            tam_storage::rule_capture::prospective_output(
                &run.pool,
                run.org,
                product,
                tam_storage::rule_capture::PricingScope::CrossList(target),
            )
            .await?,
        ),
    };
    let mapping = tam_domain::Mapping {
        id: MappingId(fresh_uuid()),
        org: run.org,
        product: product.id,
        inventory: target,
        binding: tam_domain::Binding::Unbound,
        policies: tam_domain::FieldPolicies {
            title: tam_domain::FieldPolicy::Managed,
            description: tam_domain::FieldPolicy::Managed,
            price: tam_domain::FieldPolicy::Managed,
            taxonomy: tam_domain::FieldPolicy::Managed,
            grades: tam_domain::FieldPolicy::Managed,
            files: tam_domain::FieldPolicy::Managed,
        },
        price_rule: tam_types::PriceRule::Explicit(
            output.as_ref().map_or(prepared.price, |value| value.price),
        ),
        publish: tam_domain::PublishMode::DryRun,
        lifecycle: tam_marketplace::RemoteLifecycle::Absent,
    };

    // Project once, immediately: the gaps raise their items the moment the
    // import is applied, which is what makes the import's own report the
    // drain measurement.
    let taxonomy = TaxonomyRepo::new(run.pool.clone());
    let terms = taxonomy.terms().await?;
    let target_edges = taxonomy
        .edges_into_all(&tam_taxonomy::projection_vocabularies(target, product))
        .await?;
    let no_counterparts = taxonomy.no_counterparts_into(target).await?;
    let elections = ElectionRepo::new(run.pool.clone());
    let rules = elections.rules(run.org).await?;
    // A freshly minted product has settled nothing, and the read is here
    // anyway so the two projection sites stay one shape rather than two.
    let settled = elections.answered_for(run.org, product.id).await?;
    // This projection is outbound — the imported product into `target` — so
    // the seller's own mapping decisions apply to it exactly as they apply to
    // a sync run's.
    let overrides = OverrideRepo::new(run.pool.clone()).for_org(run.org).await?;
    let outcome = project_listing_with_overrides(
        product,
        &ListingContext {
            org: run.org,
            mapping: mapping.id,
            inventory: target,
            now: run.now,
            terms: &terms,
            edges: &target_edges,
            no_counterparts: &no_counterparts,
            rules: &rules,
            settled: &settled,
        },
        &overrides,
        output.as_ref(),
    );
    Ok(PreparedTarget { mapping, outcome })
}

/// Writes what the prepared projection measured, through the caller's
/// transaction: the losses either arm recorded, and one reconciliation item
/// per gap where it blocked.
#[expect(
    clippy::too_many_lines,
    reason = "the projection's blocked arms are one match over a closed error type; splitting them would put the gate's own vocabulary in two places"
)]
async fn apply_projection(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    target: &PreparedTarget,
    now: Timestamp,
) -> Result<Projected, ImportError> {
    let (projectable, blocked_by, raised) = match &target.outcome {
        Ok(projection) => {
            record_losses(tx, org, target, &projection.loss, now).await?;
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
            record_losses(tx, org, target, loss, now).await?;
            let causes: Vec<(CanonicalTermId, tam_domain::TermKind)> = gaps
                .iter()
                .map(|gap| {
                    let tam_domain::VocabularyId(_, kind) = gap.target;
                    (gap.term, kind)
                })
                .collect();
            let raised = raise_taxonomy_gaps(
                tx,
                org,
                RaiseScope {
                    mapping: target.mapping.id,
                    target: target.mapping.inventory,
                    at: now,
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
        Err(tam_domain::ProjectionBlocked::CurrencyMismatch { .. }) => (
            false,
            Some("currency_mismatch".to_owned()),
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
    Ok(Projected {
        projectable,
        blocked_by,
        raised,
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
        .mapping_seeds(run.org, target_of(run)?, mappings)
        .await?;
    let to = match intent {
        tam_storage::SyncIntent::Draft => ListingState::Draft,
        tam_storage::SyncIntent::Live => ListingState::Live,
    };
    let target = target_of(run)?;
    let mut items: Vec<tam_storage::NewJobItem> = Vec::new();
    for seed in &seeds {
        for operation in tam_storage::lower(to, target, seed).map_err(ImportError::Lowering)? {
            items.push(tam_storage::NewJobItem {
                item: tam_domain::JobItemId(fresh_uuid()),
                mapping: seed.mapping,
                idempotency_key: tam_marketplace::idempotency::derive_idempotency_key(
                    run.org,
                    target,
                    seed.product,
                    INTENT_VERSION,
                    tam_storage::job_reads::intent_digest(
                        &operation,
                        job,
                        &seed.payload_hashes,
                        seed.sever_generation,
                    ),
                ),
                requires_bound_on: tam_storage::requires_bound_on(&operation, target),
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
pub fn resolve_price(
    source: InventoryId,
    price: &ImportedPrice,
) -> Result<PriceIntent, ImportError> {
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

/// Preserve unclassified values and native resource types. Resource types may
/// have a canonical projection, but conditions still need the original ID.
fn residue_of(listing: &tam_marketplace::ImportedListing) -> Vec<ImportedTerm> {
    listing
        .native
        .iter()
        .filter(|term| term.kind.is_none() || term.kind == Some(TermKind::ResourceType))
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
        .map_or(InventoryId::Tes, |term| term.inventory)
}

/// What the import's own projection could not carry, recorded against the
/// mapping the same transaction is minting.
///
/// The import projects once immediately, which is what makes its report the
/// drain measurement; the losses that projection measured belong to the same
/// record, so a Tes-to-TPT licence drop is visible from the moment the product
/// exists rather than only after a sync run has leased it.
async fn record_losses(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    target: &PreparedTarget,
    losses: &[tam_domain::equivalence::Loss],
    now: Timestamp,
) -> Result<(), ImportError> {
    if losses.is_empty() {
        return Ok(());
    }
    record_mapping_losses(
        tx,
        tam_storage::LossScope {
            org,
            mapping: target.mapping.id,
            attempt: tam_types::AttemptId(fresh_uuid()),
            at: now,
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
            resolve_price(InventoryId::Tes, &paid(450, "GBP")).ok(),
            Money::new(450, Currency::Gbp).ok().map(PriceIntent::Paid),
            "the write side already mints from currency_rule, and the read side agrees"
        );
        assert_eq!(
            resolve_price(InventoryId::Tpt, &paid(450, "USD")).ok(),
            Money::new(450, Currency::Usd).ok().map(PriceIntent::Paid),
            "TPT fixes USD; a rule that said every inventory mints GBP would be wrong here"
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
                resolve_price(InventoryId::Tes, &paid(450, "USD")),
                Err(ImportError::Price(_))
            ),
            "reading a USD price into a GBP-fixed inventory would silently redenominate it"
        );
    }
}
