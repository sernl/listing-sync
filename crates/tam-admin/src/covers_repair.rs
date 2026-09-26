//! `covers repair`: redraws every thumbnail that is still a generated card,
//! from the resource's own first file, where this server holds that file.
//!
//! The cards were all a resource got before covers could be drawn from a PDF
//! page. The new renderer draws them, but a stored cover does not redraw
//! itself, so this pass does it once, tenant by tenant. It is the
//! `redraw_cover` step the replace route takes, run over the catalogue: read
//! the first payload back from the object store, render it through
//! `tam_pipeline::render::cover`, store the result under the tenant's sink,
//! and offer it through `tam_storage::offer_cover` with the renderer's own
//! `is_generated_card` as the replaceability test. A cover that is anything
//! but a card is never touched, and a replayed pass writes nothing.
//!
//! A resource whose first file is a marketplace locator rather than bytes this
//! server holds is counted and left: those files live on the seller's device,
//! and the device's next import offers the new cover through the same
//! predicate.

use std::collections::BTreeMap;

use tam_pipeline::pipeline::BlobSink as _;
use tam_pipeline::render::{self, CoverSource};
use tam_storage::{BlobRepo, CoverOffer, ProductRepo, TenantBlobSink};
use tam_types::{
    FileBytes, FileId, FileKind, FileRole, OrgId, ProductFile, ProductId, ScanOutcome, Timestamp,
    Uuid,
};

/// Where the pass reads files and writes covers. Absent on a dry run, which
/// only reads the catalogue.
pub(crate) struct Store {
    pub(crate) kek: tam_secrets::Kek,
    pub(crate) backend: tam_blob_store::BlobBackend,
}

/// What one pass found and did, summed over every tenant it visited.
#[derive(Default)]
pub(crate) struct Tally {
    tenants: u64,
    /// Live resources whose cover is a generated card, old or new.
    cards: u64,
    /// Cards whose first payload file this server holds, clean, by kind:
    /// the ones a pass can redraw.
    redrawable: BTreeMap<&'static str, u64>,
    /// Cards whose first file is a marketplace locator, or which have no
    /// payload at all: the seller's device repairs these on re-import.
    not_held: u64,
    /// Cards whose first file is held but not scanned clean.
    not_clean: u64,
    /// Redrawn and swapped in.
    repaired: u64,
    /// Redrawn, but the renderer still found nothing to draw but a card.
    still_card: u64,
    /// Offered, and the storage layer kept what it had.
    kept: u64,
}

fn kind_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Pdf => "pdf",
        FileKind::Image => "image",
        FileKind::Zip => "zip",
        FileKind::Docx => "docx",
        FileKind::Pptx => "pptx",
    }
}

impl Tally {
    pub(crate) fn report(&self, dry_run: bool) -> String {
        let redrawable: u64 = self.redrawable.values().sum();
        let by_kind = if self.redrawable.is_empty() {
            String::new()
        } else {
            let kinds = self
                .redrawable
                .iter()
                .map(|(kind, count)| format!("{kind} {count}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(" ({kinds})")
        };
        let head = format!(
            "covers repair{}: {} tenant(s), {} generated card(s); {redrawable} redrawable \
             from a held file{by_kind}; {} whose file is on the seller's device; {} whose \
             file is not scanned clean",
            if dry_run { " (dry run)" } else { "" },
            self.tenants,
            self.cards,
            self.not_held,
            self.not_clean,
        );
        if dry_run {
            head
        } else {
            format!(
                "{head}\nrepaired {}; still a card after redrawing {}; kept by storage {}",
                self.repaired, self.still_card, self.kept
            )
        }
    }
}

/// The candidate a card-covered product offers, or why it offers none.
enum Candidate {
    Held {
        kind: FileKind,
        hash: tam_types::ContentHash,
    },
    NotHeld,
    NotClean,
}

fn candidate(first: Option<&ProductFile>) -> Candidate {
    match first.map(|file| (file.kind, &file.bytes)) {
        Some((
            kind,
            FileBytes::Held {
                hash,
                scan: ScanOutcome::Clean { .. },
                ..
            },
        )) => Candidate::Held { kind, hash: *hash },
        Some((_, FileBytes::Held { .. })) => Candidate::NotClean,
        Some((_, FileBytes::Sourced { .. })) | None => Candidate::NotHeld,
    }
}

/// One pass over `tenants`. A dry run needs no store; a real one does.
pub(crate) async fn run(
    pool: &sqlx::PgPool,
    tenants: &[OrgId],
    store: Option<&Store>,
    now: Timestamp,
) -> Result<Tally, Box<dyn std::error::Error>> {
    let products = ProductRepo::new(pool.clone());
    let mut tally = Tally::default();
    for &org in tenants {
        tally.tenants += 1;
        let ids: Vec<ProductId> = products
            .list(org)
            .await?
            .into_iter()
            .map(|summary| summary.id)
            .collect();
        for stored in products.covers(org, &ids).await? {
            if !render::is_generated_card(stored.hash) {
                continue;
            }
            tally.cards += 1;
            let Some(record) = products.get(org, stored.product).await? else {
                continue;
            };
            let (kind, hash) = match candidate(record.product.payload_files().next()) {
                Candidate::Held { kind, hash } => (kind, hash),
                Candidate::NotHeld => {
                    tally.not_held += 1;
                    continue;
                }
                Candidate::NotClean => {
                    tally.not_clean += 1;
                    continue;
                }
            };
            *tally.redrawable.entry(kind_name(kind)).or_default() += 1;
            if let Some(store) = store {
                redraw(
                    pool,
                    store,
                    org,
                    stored.product,
                    kind,
                    hash,
                    now,
                    &mut tally,
                )
                .await?;
            }
        }
    }
    Ok(tally)
}

/// The swap for one product: render, store, offer.
#[expect(
    clippy::too_many_arguments,
    reason = "the pool, the store, the tenant, the product, the file's kind and digest, the \
              instant and the tally; a struct over them would only name this call"
)]
async fn redraw(
    pool: &sqlx::PgPool,
    store: &Store,
    org: OrgId,
    product: ProductId,
    kind: FileKind,
    hash: tam_types::ContentHash,
    now: Timestamp,
    tally: &mut Tally,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo = BlobRepo::new(
        pool.clone(),
        store.backend.object_store(),
        store.kek.clone(),
    );
    let bytes = repo.get(org, hash).await?;
    let drawn = render::cover(kind, &bytes)?;
    if matches!(drawn.source, CoverSource::Generated { .. }) {
        tally.still_card += 1;
        return Ok(());
    }
    let byte_len = drawn.image.png.len() as u64;
    let sink = TenantBlobSink {
        repo: &repo,
        org,
        at: now,
    };
    let stored = sink.store(drawn.image.png).await?;
    let cover = ProductFile {
        id: FileId(Uuid(*uuid::Uuid::new_v4().as_bytes())),
        role: FileRole::Cover,
        kind: FileKind::Image,
        bytes: FileBytes::Held {
            hash: stored,
            byte_len,
            // Rendered here from bytes this server scanned clean at upload,
            // the verdict `redraw_cover` records for the same step.
            scan: ScanOutcome::Clean { at: now },
        },
    };
    let mut tx = pool.begin().await?;
    // The pin `tam_storage` applies inside its own repositories: the
    // transaction is this pass's, so the pin is too.
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await?;
    let offered = tam_storage::offer_cover(
        &mut tx,
        org,
        product,
        &cover,
        render::is_generated_card,
        now,
    )
    .await?;
    tx.commit().await?;
    match offered {
        CoverOffer::Written | CoverOffer::Replaced => tally.repaired += 1,
        CoverOffer::AlreadyHeld | CoverOffer::NoProduct => tally.kept += 1,
    }
    Ok(())
}
