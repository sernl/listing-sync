//! Authoring: the byte upload, the create, the edit and the delete.
//!
//! The gap this closes is authoring and only authoring. Everything
//! downstream already exists — `POST /{version}/jobs` enqueues, the engine's
//! pump executes, the ledger reports — so nothing here performs a marketplace
//! write or enqueues one, with the single exception of the removal legs a
//! delete elects, which are ordinary job items on the existing path.
//!
//! Bytes travel once, at [`upload`], through the same `tam_pipeline::ingest`
//! and `TenantBlobSink` the operator import runs, so every guard the import
//! has an upload has too: the kind probe, the malware scan before extraction,
//! the archive bounds, the generated cover, and per-tenant sealing. The
//! create then names the handles that upload returned, which is why a
//! payload-less create is refused here rather than at the deferred
//! `assert_product_has_payload` trigger, where it would surface as a 500.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_domain::equivalence::ElectionTriggerKind;
use tam_domain::registry::registry;
use tam_domain::{
    Binding, CanonicalProduct, DeclarationSource, FieldPolicies, FieldPolicy, GradeDeclaration,
    JobItemId, Mapping, PublishMode, RightsDeclaration, TermKind, VocabularyId, VocabularyPath,
};
use tam_marketplace::idempotency::derive_idempotency_key;
use tam_marketplace::{ListingState, RemoteLifecycle, RemoteListingId};
use tam_pipeline::archive::ExtractBudget;
use tam_pipeline::pipeline::{ingest, ArchiveMode, IngestContext, IngestError};
use tam_pipeline::scan::EicarScanner;
use tam_pipeline::store::LocalObjectStore;
use tam_storage::{
    intent_digest, AnsweredElection, BlobRepo, ElectionRepo, JobRepo, MappingAdd, MappingRecord,
    MappingRepo, NewJob, NewJobItem, ProductEdit, ProductRepo, StorageError, TenantBlobSink,
    TptBaseRepo,
};
use tam_types::{
    Actor, CanonicalTermId, ContentHash, CopyFormat, FileId, FileRole, InventoryId, JobId,
    ListingCopy, MappingId, Money, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile,
    ProductId, ScanOutcome, Stamp, Timestamp, Title, Uuid,
};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::product::{record_of, verdict, DraftHead, TptBaseInput};
use crate::quota::{quota_for, QuotaKind};
use crate::resources::{kind_from_str, kind_str, MappingHeadView};
use crate::{AppState, OrgContext};

/// The intent version the item idempotency key is derived under, matching
/// `jobs.rs`: one vocabulary, so a removal enqueued here and a create
/// enqueued there cannot collide by deriving under different versions.
const INTENT_VERSION: u32 = 1;

/// The leg name the removal job's request key is derived from, so a repeated
/// delete replays the first job rather than minting a second.
const DELETE_LEG: &str = "product-delete";

// ------------------------------------------------------------------ errors

fn storage_fault(state: &AppState, error: &StorageError) -> APIError {
    state.internal(&error.to_string())
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

fn coded(status: StatusCode, message: &str, code: APIErrorCode) -> APIError {
    APIError::new(
        status,
        APIErrorEntry::new(message)
            .code(code)
            .kind(APIErrorKind::Validation),
    )
}

fn missing(what: &str) -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new(what)
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
    )
}

fn parse_product_id(raw: &str) -> Result<ProductId, APIError> {
    Uuid::parse_hyphenated(raw)
        .map(ProductId)
        .ok_or_else(|| validation("the identifier is not a UUID"))
}

/// One byte handle as the upload returns it and the create names it back.
///
/// The hash is the whole handle: bytes are content-addressed per tenant, so a
/// hash this organisation has never stored resolves to no blob and the create
/// is refused. `kind` and `byte_len` ride along because the create writes
/// them onto the `product_file` row, and `byte_len` is checked against the
/// stored blob rather than trusted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHandle {
    /// The blake3 content hash, lowercase hex.
    pub hash: String,
    /// `pdf`, `pptx`, `docx`, `zip` or `image`, spelled as the product view
    /// spells it.
    pub kind: String,
    pub byte_len: u64,
}

impl FileHandle {
    fn of(file: &tam_pipeline::pipeline::IngestedFile) -> Self {
        Self {
            hash: hex_encode(&file.hash.0),
            kind: kind_str(file.kind).to_owned(),
            byte_len: file.byte_len,
        }
    }

    fn resolve(&self, role: FileRole, now: Timestamp) -> Result<ProductFile, APIError> {
        let hash = parse_hash(&self.hash)
            .ok_or_else(|| validation("a file handle's hash is not a 64-character hex digest"))?;
        let kind = kind_from_str(&self.kind)
            .ok_or_else(|| validation("a file handle names a kind this server does not store"))?;
        Ok(ProductFile {
            id: FileId(fresh_uuid()),
            role,
            kind,
            hash,
            byte_len: self.byte_len,
            // The pipeline scanned the bytes before they were stored, and a
            // handle exists only because that scan passed.
            scan: ScanOutcome::Clean { at: now },
        })
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    use core::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // infallible on String; the Result is the trait's, not the writer's
        let _unused: core::fmt::Result = write!(out, "{byte:02x}");
    }
    out
}

fn parse_hash(raw: &str) -> Option<ContentHash> {
    if raw.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, slot) in bytes.iter_mut().enumerate() {
        let pair = raw.get(index * 2..index * 2 + 2)?;
        *slot = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(ContentHash(bytes))
}

/// The one place this module mints row identity; v4 through the generator the
/// workspace already trusts for it.
fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}

// ------------------------------------------------------------------ upload

/// How an archive upload is treated, as the query parameter spells it.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveParam {
    /// One payload file per recognised entry, which is what the import does.
    #[default]
    Explode,
    /// One payload file, whatever the upload's kind. The mode a bundle
    /// destined for TPT needs, because a TPT create takes exactly one file.
    KeepWhole,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct UploadParams {
    #[serde(default)]
    pub archive: ArchiveParam,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedView {
    pub payload: Vec<FileHandle>,
    pub cover: FileHandle,
    pub previews: Vec<FileHandle>,
    /// This tenant's blob bytes after the upload landed, and the ceiling they
    /// are measured against, so a client can render the headroom it has left
    /// without a second call.
    pub stored_bytes: i64,
    pub storage_bytes_max: u64,
}

fn ingest_refusal(error: &IngestError) -> APIError {
    let message = error.to_string();
    match error {
        IngestError::UnknownKind
        | IngestError::Infected { .. }
        | IngestError::Archive(_)
        | IngestError::Cover(_)
        | IngestError::EmptyPayload => coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            &message,
            APIErrorCode::UploadRejected,
        ),
        // A store fault is ours, not the seller's, and its internals are
        // withheld under the same disclosure rule as any other.
        IngestError::Store(_) => APIError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            APIErrorEntry::new("the upload could not be stored")
                .code(APIErrorCode::Internal)
                .kind(APIErrorKind::Internal),
        ),
    }
}

pub(crate) async fn upload(
    State(state): State<AppState>,
    context: OrgContext,
    Query(params): Query<UploadParams>,
    body: Bytes,
) -> Result<(StatusCode, Json<UploadedView>), APIError> {
    let blobs = state.blobs.clone().ok_or_else(|| {
        APIError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            APIErrorEntry::new(
                "this deployment holds no key-encryption key or object-store root; \
                 no bytes were accepted",
            )
            .code(APIErrorCode::BlobStoreUnavailable)
            .kind(APIErrorKind::Internal),
        )
    })?;
    if body.is_empty() {
        return Err(coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            "the upload carried no bytes",
            APIErrorCode::UploadRejected,
        ));
    }
    let now = (state.wall)();
    let products = ProductRepo::new(state.pool.clone());
    let quota = quota_for(&state, context.org).await?;
    let used = products
        .stored_bytes(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let incoming = i64::try_from(body.len()).unwrap_or(i64::MAX);
    // Checked against the upload's own size before a byte is sealed. An
    // exploding archive writes more than it arrived as, so this bounds the
    // dominant term rather than the exact one; the exact total is reported
    // back below and the next upload is refused against it.
    if used.saturating_add(incoming) > i64::try_from(quota.storage_bytes_max).unwrap_or(i64::MAX) {
        return Err(quota_refusal(
            QuotaKind::StorageBytes,
            used,
            quota.storage_bytes_max,
        ));
    }

    let repo = BlobRepo::new(
        state.pool.clone(),
        LocalObjectStore::new(blobs.root.clone()),
        blobs.kek.clone(),
    );
    let sink = TenantBlobSink {
        repo: &repo,
        org: context.org,
        at: now,
    };
    let ingested = ingest(
        &body,
        &EicarScanner,
        &sink,
        IngestContext {
            budget: ExtractBudget::default(),
            now,
            archives: match params.archive {
                ArchiveParam::Explode => ArchiveMode::Explode,
                ArchiveParam::KeepWhole => ArchiveMode::KeepWhole,
            },
        },
    )
    .await
    .map_err(|error| ingest_refusal(&error))?;

    let stored_bytes = products
        .stored_bytes(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok((
        StatusCode::CREATED,
        Json(UploadedView {
            payload: ingested.payload.iter().map(FileHandle::of).collect(),
            cover: FileHandle::of(&ingested.cover),
            previews: ingested.previews.iter().map(FileHandle::of).collect(),
            stored_bytes,
            storage_bytes_max: quota.storage_bytes_max,
        }),
    ))
}

fn quota_refusal(kind: QuotaKind, used: i64, limit: u64) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(kind.message())
            .code(APIErrorCode::QuotaExceeded)
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({
                "quota": kind.as_str(),
                "used": used,
                "limit": limit,
            })),
    )
}

// ------------------------------------------------------------------ create

/// One vocabulary value as the create and the edit name it, matching the
/// shape [`PathView`] reads back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathInput {
    pub inventory: InventoryId,
    pub kind: TermKind,
    pub segments: Vec<String>,
    #[serde(default)]
    pub native_id: Option<String>,
}

impl PathInput {
    fn resolve(&self) -> Result<VocabularyPath, APIError> {
        if self.segments.is_empty() {
            return Err(validation("a vocabulary value needs at least one segment"));
        }
        Ok(VocabularyPath {
            vocabulary: VocabularyId(self.inventory, self.kind),
            segments: self.segments.clone(),
            native_id: self.native_id.clone(),
        })
    }
}

/// The rights grant the seller stated, kept as the source's own value. The
/// axis is not carried: a rights declaration is a licence by construction and
/// a field that could disagree with that is a field that will.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RightsInput {
    pub inventory: InventoryId,
    pub segments: Vec<String>,
    #[serde(default)]
    pub native_id: Option<String>,
}

/// One answer the seller gave on the form, before any mapping exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElectionInput {
    pub inventory: InventoryId,
    pub axis: TermKind,
    /// `supply`, `elect_one`, `over_cap` or `narrow`.
    pub trigger: String,
    /// `free` or `paid` for a supply, the source value's native id for a
    /// narrow, absent for the two kinds that generalise to nothing.
    #[serde(default)]
    pub trigger_key: Option<String>,
    pub answers: Vec<AnswerInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerInput {
    pub segments: Vec<String>,
    #[serde(default)]
    pub native_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateProductBody {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default = "markdown")]
    pub body_format: CopyFormat,
    pub price: PriceIntent,
    /// The handles `POST /uploads` returned. At least one, because a product
    /// with no payload cannot be listed anywhere and the database says so at
    /// commit.
    pub payload: Vec<FileHandle>,
    #[serde(default)]
    pub cover: Option<FileHandle>,
    #[serde(default)]
    pub previews: Vec<FileHandle>,
    #[serde(default)]
    pub subjects: Vec<CanonicalTermId>,
    #[serde(default)]
    pub grades: Vec<PathInput>,
    #[serde(default)]
    pub rights: Option<RightsInput>,
    /// The platforms this product is authored for. One `Unbound`, `DryRun`
    /// mapping per entry, and nothing is enqueued: publishing is a second,
    /// deliberate action on `POST /{version}/jobs`.
    #[serde(default)]
    pub inventories: Vec<InventoryId>,
    #[serde(default)]
    pub elections: Vec<ElectionInput>,
    /// The TPT-base fields `product` has no column for, which land in the
    /// sidecar migration 0040 declares. Absent is a create from a path that
    /// does not carry them — the operator import is one — and writes no row,
    /// which is why every read of that table is an outer join.
    #[serde(default)]
    pub tpt_base: Option<TptBaseInput>,
}

const fn markdown() -> CopyFormat {
    CopyFormat::Markdown
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedProductView {
    pub product: ProductId,
    pub mappings: Vec<MappingView>,
    /// How many already-answered elections were written. Reported rather than
    /// echoed: a question this product had already settled writes no second
    /// answer, and the seller is told rather than left to assume.
    pub elections_recorded: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MappingView {
    pub inventory: InventoryId,
    pub mapping: MappingId,
}

/// Re-validates a price through the smart constructor, which deserialisation
/// bypasses: `Money::new` is where a non-positive paid amount is refused, and
/// a body that reached the handler has not been through it.
fn checked_price(price: PriceIntent) -> Result<PriceIntent, APIError> {
    match price {
        PriceIntent::Free => Ok(PriceIntent::Free),
        PriceIntent::Paid(money) => Money::new(money.minor_units(), money.currency())
            .map(PriceIntent::Paid)
            .map_err(|error| {
                validation(&format!(
                    "the price is not one a listing can carry: {error:?}"
                ))
            }),
    }
}

/// Whether every field the selected platforms declare required is answered.
///
/// Requiredness is reported as the registry has it and never invented: Tes
/// `licence` is the only field declared required anywhere, because it is the
/// only refusal anyone has measured. A licence reaches the write either as
/// the product's own rights declaration or as an already-answered licence
/// election for that inventory, so either satisfies it.
fn required_fields_answered(
    inventories: &[InventoryId],
    rights: Option<&RightsInput>,
    elections: &[ElectionInput],
) -> Result<(), APIError> {
    let mut unmet: Vec<serde_json::Value> = Vec::new();
    for inventory in inventories {
        for native in registry(*inventory).natives {
            if !native.required {
                continue;
            }
            let axis = registry(*inventory)
                .equivalence_axes
                .iter()
                .find(|binding| binding.native == native.name)
                .map(|binding| binding.axis);
            let answered = match axis {
                Some(TermKind::Licence) => {
                    rights.is_some()
                        || elections.iter().any(|election| {
                            election.inventory == *inventory
                                && election.axis == TermKind::Licence
                                && !election.answers.is_empty()
                        })
                }
                // No other required field exists in the registry today. A new
                // one arrives unanswerable rather than silently satisfied,
                // which is the honest default: the form has to be taught it.
                Some(_) | None => false,
            };
            if !answered {
                unmet.push(serde_json::json!({
                    "inventory": inventory,
                    "field": native.name,
                }));
            }
        }
    }
    if unmet.is_empty() {
        return Ok(());
    }
    Err(APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new("a selected platform requires a field this product does not carry")
            .code(APIErrorCode::RequiredFieldMissing)
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({ "missing": unmet })),
    ))
}

fn trigger_kind_of(raw: &str) -> Option<ElectionTriggerKind> {
    ElectionTriggerKind::ALL
        .into_iter()
        .find(|kind| kind.as_str() == raw)
}

/// The draft the model validates, assembled from the create body's own fields
/// and the TPT-base block beside them.
///
/// One source per field: the title, description, price, payload handle and
/// grades come from the body that already carries them, and the block carries
/// only what `product` has no column for. Sending either twice is how two
/// copies come to disagree.
fn draft_head(
    title: &str,
    body: &str,
    price: PriceIntent,
    payload: &[FileHandle],
    grades: &[PathInput],
) -> DraftHead {
    DraftHead {
        name: title.to_owned(),
        description: body.to_owned(),
        free: matches!(price, PriceIntent::Free),
        price_minor_units: match price {
            PriceIntent::Free => None,
            PriceIntent::Paid(money) => Some(money.minor_units()),
        },
        payload_hash: payload.first().map(|handle| handle.hash.clone()),
        grades: grades
            .iter()
            .map(|path| {
                path.native_id
                    .clone()
                    .or_else(|| path.segments.first().cloned())
                    .unwrap_or_default()
            })
            .collect(),
    }
}

/// Everything the model refuses about this draft, as the seller's own
/// refusals rather than a database fault.
///
/// The same function `POST /{version}/authoring/check` answers with, so the
/// endpoint that reports a refusal and the endpoint that acts on one cannot
/// come to different conclusions.
fn refuse_unsubmittable(draft: &crate::product::DraftInput) -> Result<(), APIError> {
    let view = verdict(draft);
    if view.submittable {
        return Ok(());
    }
    Err(APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new("the product does not satisfy the create form's own rules")
            .code(APIErrorCode::RequiredFieldMissing)
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({ "refusals": view.refusals })),
    ))
}

pub(crate) async fn create_product(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<CreateProductBody>,
) -> Result<(StatusCode, Json<CreatedProductView>), APIError> {
    if body.title.trim().is_empty() {
        return Err(validation("a product needs a title"));
    }
    // Refused here rather than at commit. The deferred
    // `assert_product_has_payload` trigger states the same invariant, and
    // letting it fire would turn a form the seller can fix into a 500.
    let Some((head, rest)) = body.payload.split_first() else {
        return Err(coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            "a product needs at least one payload file; upload the bytes first",
            APIErrorCode::PayloadMissing,
        ));
    };
    let price = checked_price(body.price)?;
    required_fields_answered(&body.inventories, body.rights.as_ref(), &body.elections)?;
    // The TPT-base block is validated before anything is written, against the
    // same rules the form ran inline and the check endpoint answers with. A
    // create that carries no block is one from a path that does not author on
    // this form, and it is left alone rather than being refused for missing
    // controls it never had.
    let sidecar = match body.tpt_base.clone() {
        None => None,
        Some(base) => {
            let draft = base.into_draft(draft_head(
                &body.title,
                &body.body,
                price,
                &body.payload,
                &body.grades,
            ));
            refuse_unsubmittable(&draft)?;
            Some(record_of(&draft)?)
        }
    };

    let now = (state.wall)();
    let products = ProductRepo::new(state.pool.clone());
    let quota = quota_for(&state, context.org).await?;
    let live = products
        .live_count(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if live >= i64::from(quota.listings_max) {
        return Err(quota_refusal(
            QuotaKind::Listings,
            live,
            u64::from(quota.listings_max),
        ));
    }

    // Every handle must name bytes this tenant has actually uploaded. The
    // catalogue insert upserts a blob row rather than requiring one, so a
    // fabricated hash would otherwise mint a row pointing at no object and
    // charge the tenant for storage that does not exist.
    let claimed: Vec<ContentHash> = body
        .payload
        .iter()
        .chain(body.cover.iter())
        .chain(body.previews.iter())
        .map(|handle| {
            parse_hash(&handle.hash)
                .ok_or_else(|| validation("a file handle's hash is not a 64-character hex digest"))
        })
        .collect::<Result<Vec<_>, APIError>>()?;
    let known = products
        .stored_hashes(context.org, &claimed)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let unknown: Vec<String> = claimed
        .iter()
        .filter(|hash| !known.contains(hash))
        .map(|hash| hex_encode(&hash.0))
        .collect();
    if !unknown.is_empty() {
        return Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new("a file handle names bytes this organisation has not uploaded")
                .code(APIErrorCode::UploadRejected)
                .kind(APIErrorKind::Validation)
                .detail(serde_json::json!({ "hashes": unknown })),
        ));
    }

    let payload = PayloadSet::new(
        head.resolve(FileRole::Payload, now)?,
        rest.iter()
            .map(|handle| handle.resolve(FileRole::Payload, now))
            .collect::<Result<Vec<_>, APIError>>()?,
    );
    let cover = body
        .cover
        .as_ref()
        .map(|handle| handle.resolve(FileRole::Cover, now))
        .transpose()?;
    let previews = body
        .previews
        .iter()
        .map(|handle| handle.resolve(FileRole::Preview, now))
        .collect::<Result<Vec<_>, APIError>>()?;

    let raw: Vec<VocabularyPath> = body
        .grades
        .iter()
        .map(PathInput::resolve)
        .collect::<Result<Vec<_>, APIError>>()?;
    let grades = GradeDeclaration {
        source: DeclarationSource::Seller,
        derived: tam_taxonomy::derive_interval(&raw),
        raw,
    };
    let rights = match &body.rights {
        None => RightsDeclaration::Unstated,
        Some(input) => {
            if input.segments.is_empty() {
                return Err(validation(
                    "a stated rights grant needs at least one segment",
                ));
            }
            RightsDeclaration::Declared {
                source: VocabularyPath {
                    vocabulary: VocabularyId(input.inventory, TermKind::Licence),
                    segments: input.segments.clone(),
                    native_id: input.native_id.clone(),
                },
            }
        }
    };

    let product = ProductId(fresh_uuid());
    products
        .insert(
            context.org,
            &CanonicalProduct {
                id: product,
                org: context.org,
                title: Title(body.title.clone()),
                body: ListingCopy {
                    body: body.body.clone(),
                    format: body.body_format,
                },
                payload,
                cover,
                previews,
                subjects: body.subjects.clone(),
                grades,
                price,
                rights,
                // Nothing authored here came off a marketplace, so there is no
                // source value in an untyped axis to keep.
                native_residue: vec![],
            },
            now,
        )
        .await
        .map_err(|error| create_fault(&state, &error))?;

    if let Some(record) = &sidecar {
        // After the product row, because the sidecar's foreign key names it.
        TptBaseRepo::new(state.pool.clone())
            .upsert(context.org, product, record, now)
            .await
            .map_err(|error| create_fault(&state, &error))?;
    }

    let mappings = MappingRepo::new(state.pool.clone());
    let mut written = Vec::with_capacity(body.inventories.len());
    for inventory in &body.inventories {
        let mapping = MappingId(fresh_uuid());
        mappings
            .insert(
                context.org,
                &unbound_mapping(context.org, product, *inventory, mapping, price),
                0,
                now,
            )
            .await
            .map_err(|error| storage_fault(&state, &error))?;
        written.push(MappingView {
            inventory: *inventory,
            mapping,
        });
    }

    let elections = ElectionRepo::new(state.pool.clone());
    let mut recorded = 0usize;
    for election in &body.elections {
        let kind = trigger_kind_of(&election.trigger).ok_or_else(|| {
            validation("an election trigger is supply, elect_one, over_cap or narrow")
        })?;
        if election.answers.is_empty() {
            return Err(validation("an answered election names at least one value"));
        }
        let vocabulary = VocabularyId(election.inventory, election.axis);
        let paths: Vec<VocabularyPath> = election
            .answers
            .iter()
            .map(|answer| {
                if answer.segments.is_empty() {
                    return Err(validation(
                        "an answered value needs at least one path segment",
                    ));
                }
                Ok(VocabularyPath {
                    vocabulary,
                    segments: answer.segments.clone(),
                    native_id: answer.native_id.clone(),
                })
            })
            .collect::<Result<Vec<_>, APIError>>()?;
        let wrote = elections
            .record_answered(
                context.org,
                &AnsweredElection {
                    product,
                    inventory: election.inventory,
                    axis: election.axis,
                    trigger_kind: kind,
                    trigger_key: election.trigger_key.as_deref(),
                    paths: &paths,
                },
                now,
            )
            .await
            .map_err(|error| match error {
                StorageError::Inconsistent { ref reason } => validation(reason),
                other @ (StorageError::Db(_)
                | StorageError::TimestampOutOfRange { .. }
                | StorageError::CorruptRow { .. }
                | StorageError::OrgMismatch
                | StorageError::StaleLease
                | StorageError::DuplicateIdempotencyKey { .. }
                | StorageError::AttemptInFlight
                | StorageError::MappingAlreadyBound
                | StorageError::ListingAlreadyBound) => storage_fault(&state, &other),
            })?;
        recorded += usize::from(wrote);
    }

    Ok((
        StatusCode::CREATED,
        Json(CreatedProductView {
            product,
            mappings: written,
            elections_recorded: recorded,
        }),
    ))
}

/// A product insert whose one seller-reachable failure is a handle whose
/// declared length disagrees with the blob it names, which the repository
/// reports as an inconsistency rather than a fault of ours.
fn create_fault(state: &AppState, error: &StorageError) -> APIError {
    match error {
        StorageError::Inconsistent { reason } => coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            reason,
            APIErrorCode::UploadRejected,
        ),
        other @ (StorageError::Db(_)
        | StorageError::TimestampOutOfRange { .. }
        | StorageError::CorruptRow { .. }
        | StorageError::OrgMismatch
        | StorageError::StaleLease
        | StorageError::DuplicateIdempotencyKey { .. }
        | StorageError::AttemptInFlight
        | StorageError::MappingAlreadyBound
        | StorageError::ListingAlreadyBound) => storage_fault(state, other),
    }
}

fn unbound_mapping(
    org: OrgId,
    product: ProductId,
    inventory: InventoryId,
    mapping: MappingId,
    price: PriceIntent,
) -> Mapping {
    Mapping {
        id: mapping,
        org,
        product,
        inventory,
        binding: Binding::Unbound,
        policies: FieldPolicies {
            title: FieldPolicy::Managed,
            description: FieldPolicy::Managed,
            price: FieldPolicy::Managed,
            taxonomy: FieldPolicy::Managed,
            grades: FieldPolicy::Managed,
            files: FieldPolicy::Managed,
        },
        price_rule: PriceRule::Explicit(price),
        // Never `Publish`: a create writes the catalogue and enqueues
        // nothing, and where the listing ends up is the publish control's
        // decision on the job endpoint.
        publish: PublishMode::DryRun,
        lifecycle: RemoteLifecycle::Absent,
    }
}

// ------------------------------------------------------- add a marketplace

/// Which marketplace to add. The mapping's identifier is the server's to
/// mint, and its state is not a client's to choose: it starts where a create
/// would have started it.
#[derive(Debug, Clone, Deserialize)]
pub struct AddMappingBody {
    pub inventory: InventoryId,
}

/// Adds a marketplace to a product that already exists.
///
/// The marketplaces a product reaches are chosen when its draft is created,
/// and cross-listing to one it was not created with is the action that had no
/// endpoint at all (gap G1 in `docs/notes/design/seller-dashboard.md`). The
/// mapping this mints is the create's own unbound mapping, so a send through
/// `POST /{version}/jobs` afterwards is the same path a mapping chosen at
/// create time travels; nothing here contacts a marketplace.
pub(crate) async fn add_mapping(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, product)): Path<(String, String)>,
    Json(body): Json<AddMappingBody>,
) -> Result<(StatusCode, Json<MappingHeadView>), APIError> {
    let product = parse_product_id(&product)?;
    let stored = ProductRepo::new(state.pool.clone())
        .get(context.org, product)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such product"))?;

    let now = (state.wall)();
    let mapping = MappingId(fresh_uuid());
    let repo = MappingRepo::new(state.pool.clone());
    let added = repo
        .add(
            context.org,
            &unbound_mapping(
                context.org,
                product,
                body.inventory,
                mapping,
                stored.product.price,
            ),
            0,
            now,
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if added == MappingAdd::AlreadyMapped {
        return Err(coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            "this item already reaches that marketplace",
            APIErrorCode::MappingAlreadyExists,
        ));
    }

    let head = repo
        .head(context.org, mapping)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| state.internal("the mapping just written was not readable"))?;
    Ok((StatusCode::CREATED, Json(MappingHeadView::of(head))))
}

// -------------------------------------------------------------------- edit

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PatchProductBody {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    /// Given only alongside `body`: a format that moved without the text it
    /// describes is how a Markdown listing acquires escaped markup.
    #[serde(default)]
    pub body_format: Option<CopyFormat>,
    #[serde(default)]
    pub price: Option<PriceIntent>,
    #[serde(default)]
    pub subjects: Option<Vec<CanonicalTermId>>,
    #[serde(default)]
    pub grades: Option<Vec<PathInput>>,
    #[serde(default)]
    pub rights: Option<RightsInput>,
    /// Given whole or not at all. The sidecar row is replaced rather than
    /// merged, because a field the seller cleared is a field they cleared and
    /// a merge would make clearing a control impossible to express.
    #[serde(default)]
    pub tpt_base: Option<TptBaseInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchedProductView {
    pub product: ProductId,
    /// The platforms this edit will reach when it is next synced, and the
    /// ones it will not, each with the reason. Reported so the client can say
    /// what an edit does rather than implying it reaches everywhere.
    pub reaches: Vec<InventoryId>,
}

/// Which live listings this edit cannot be attempted against.
///
/// A bound mapping lowers a revise, and the lowering refuses a transition no
/// capture supports: Tes serves neither live-to-live nor live-to-draft, so a
/// Tes listing that is already live cannot be edited through us today. The
/// refusal happens here rather than at enqueue so the seller is told before
/// the edit is written, not after an item settles.
fn uncaptured_edits(mappings: &[MappingRecord]) -> Vec<(InventoryId, &'static str)> {
    mappings
        .iter()
        .filter_map(|record| {
            let bound = matches!(record.mapping.binding, Binding::Bound { .. });
            let live = matches!(record.mapping.lifecycle, RemoteLifecycle::Live { .. });
            if !bound || !live {
                return None;
            }
            tam_storage::uncaptured_transition(
                record.mapping.inventory,
                ListingState::Live,
                ListingState::Live,
            )
            .map(|capability| (record.mapping.inventory, capability))
        })
        .collect()
}

pub(crate) async fn patch_product(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, product)): Path<(String, String)>,
    Json(body): Json<PatchProductBody>,
) -> Result<Json<PatchedProductView>, APIError> {
    let product = parse_product_id(&product)?;
    if body.body.is_none() && body.body_format.is_some() {
        return Err(validation(
            "a body format is given with the body it describes, never on its own",
        ));
    }
    let now = (state.wall)();
    let mappings = MappingRepo::new(state.pool.clone())
        .list_for_product(context.org, product)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let refused = uncaptured_edits(&mappings);
    if !refused.is_empty() {
        return Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new(
                "this listing is live on a platform whose edit-published transition is \
                 uncaptured, so the edit cannot be attempted",
            )
            .code(APIErrorCode::UncapturedTransition)
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({
                "blocked": refused
                    .iter()
                    .map(|(inventory, capability)| serde_json::json!({
                        "inventory": inventory,
                        "capability": capability,
                    }))
                    .collect::<Vec<_>>(),
            })),
        ));
    }

    let price = body.price.map(checked_price).transpose()?;
    let grades = match &body.grades {
        None => None,
        Some(paths) => {
            let raw: Vec<VocabularyPath> = paths
                .iter()
                .map(PathInput::resolve)
                .collect::<Result<Vec<_>, APIError>>()?;
            Some(GradeDeclaration {
                source: DeclarationSource::Seller,
                derived: tam_taxonomy::derive_interval(&raw),
                raw,
            })
        }
    };
    let rights = match &body.rights {
        None => None,
        Some(input) => {
            if input.segments.is_empty() {
                return Err(validation(
                    "a stated rights grant needs at least one segment",
                ));
            }
            Some(RightsDeclaration::Declared {
                source: VocabularyPath {
                    vocabulary: VocabularyId(input.inventory, TermKind::Licence),
                    segments: input.segments.clone(),
                    native_id: input.native_id.clone(),
                },
            })
        }
    };
    let edit = ProductEdit {
        title: body.title.as_ref().map(|title| Title(title.clone())),
        body: body.body.as_ref().map(|text| ListingCopy {
            body: text.clone(),
            format: body.body_format.unwrap_or(CopyFormat::Markdown),
        }),
        price,
        subjects: body.subjects.clone(),
        grades,
        rights,
    };
    let touched = ProductRepo::new(state.pool.clone())
        .update(context.org, product, &edit, now)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if !touched {
        return Err(missing("no such product"));
    }

    // The sidecar is replaced whole where the edit carries one. It is written
    // after the product update rather than before, so an edit refused as a
    // missing product leaves no orphaned row behind.
    if let Some(base) = body.tpt_base.clone() {
        let record = record_of(&base.into_draft(DraftHead {
            // The edit carries only what it changes, so the parts the model
            // validates against — the title, the price, the payload — are not
            // all present. The sidecar's own shapes are still refused by
            // `record_of`; the whole-product rules were answered at create and
            // are re-answered by the form before it sends this.
            name: body.title.clone().unwrap_or_default(),
            description: body.body.clone().unwrap_or_default(),
            free: matches!(price, Some(PriceIntent::Free) | None),
            price_minor_units: match price {
                Some(PriceIntent::Paid(money)) => Some(money.minor_units()),
                Some(PriceIntent::Free) | None => None,
            },
            payload_hash: None,
            grades: vec![],
        }))?;
        TptBaseRepo::new(state.pool.clone())
            .upsert(context.org, product, &record, now)
            .await
            .map_err(|error| storage_fault(&state, &error))?;
    }
    Ok(Json(PatchedProductView {
        product,
        reaches: mappings
            .iter()
            .map(|record| record.mapping.inventory)
            .collect(),
    }))
}

// ------------------------------------------------------------------ delete

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeleteProductBody {
    /// The platforms whose listing is to be removed as well. One removal job
    /// per entry, on the same ledger every other write travels.
    #[serde(default)]
    pub remove_from: Vec<InventoryId>,
    /// Whether a bound listing this delete does not remove may be left
    /// standing. Explicit and defaulted off, because a local delete that
    /// leaves bound mappings behind leaves live listings nobody tracks.
    #[serde(default)]
    pub leave_live: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletedProductView {
    pub product: ProductId,
    /// One entry per elected platform, naming the job whose ledger carries
    /// the removal's outcome. Failures surface there, through the item's own
    /// `failure_code`, `blocked_on` and `evidence_ref`, not on this response.
    pub removals: Vec<RemovalView>,
    /// Bound listings this delete deliberately left standing, which is only
    /// reachable with `leave_live`.
    pub left_live: Vec<InventoryId>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RemovalView {
    pub inventory: InventoryId,
    pub job: JobId,
    pub mapping: MappingId,
}

/// The listing a removal addresses and the state it is in, or `None` where
/// the mapping binds nothing to remove.
fn removal_subject(record: &MappingRecord) -> Option<(RemoteListingId, ListingState)> {
    let Binding::Bound { id, .. } = &record.mapping.binding else {
        return None;
    };
    // The engine's admission gate compares this against the mapping's own
    // observed lifecycle, so it is read off the mapping rather than assumed.
    // An unobserved lifecycle compares against nothing and admits either.
    let state = match record.mapping.lifecycle {
        RemoteLifecycle::Live { .. } => ListingState::Live,
        RemoteLifecycle::Absent
        | RemoteLifecycle::Draft
        | RemoteLifecycle::Submitted { .. }
        | RemoteLifecycle::InReview { .. }
        | RemoteLifecycle::Rejected { .. }
        | RemoteLifecycle::Withdrawn { .. } => ListingState::Draft,
    };
    Some((id.clone(), state))
}

pub(crate) async fn delete_product(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, product)): Path<(String, String)>,
    body: Option<Json<DeleteProductBody>>,
) -> Result<Json<DeletedProductView>, APIError> {
    let product = parse_product_id(&product)?;
    let Json(body) = body.unwrap_or_default();
    let now = (state.wall)();
    let mappings = MappingRepo::new(state.pool.clone())
        .list_for_product(context.org, product)
        .await
        .map_err(|error| storage_fault(&state, &error))?;

    let bound: Vec<&MappingRecord> = mappings
        .iter()
        .filter(|record| matches!(record.mapping.binding, Binding::Bound { .. }))
        .collect();
    let unelected: Vec<InventoryId> = bound
        .iter()
        .map(|record| record.mapping.inventory)
        .filter(|inventory| !body.remove_from.contains(inventory))
        .collect();
    if !unelected.is_empty() && !body.leave_live {
        return Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new(
                "this product is live on a platform this delete does not remove it from; \
                 elect the platform or state that the live listing is to be left alone",
            )
            .code(APIErrorCode::ListingStillBound)
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({ "bound": unelected })),
        ));
    }
    // Elected but never bound is a request to remove something that was never
    // created, which is a mistake worth naming rather than a silent no-op.
    let unbound: Vec<InventoryId> = body
        .remove_from
        .iter()
        .copied()
        .filter(|inventory| {
            !bound
                .iter()
                .any(|record| record.mapping.inventory == *inventory)
        })
        .collect();
    if !unbound.is_empty() {
        return Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new("no listing is bound on a platform this delete elects to remove")
                .code(APIErrorCode::SyncMappingsInvalid)
                .kind(APIErrorKind::Validation)
                .detail(serde_json::json!({ "inventories": unbound })),
        ));
    }

    let jobs = JobRepo::new(state.pool.clone());
    let mut removals = Vec::with_capacity(body.remove_from.len());
    for record in &bound {
        let inventory = record.mapping.inventory;
        if !body.remove_from.contains(&inventory) {
            continue;
        }
        let Some((subject, listing_state)) = removal_subject(record) else {
            continue;
        };
        let job = JobId(fresh_uuid());
        let operation = tam_domain::ItemOperation::Remove {
            subject,
            state: listing_state,
        };
        let item = NewJobItem {
            item: JobItemId(fresh_uuid()),
            mapping: record.mapping.id,
            idempotency_key: derive_idempotency_key(
                context.org,
                inventory,
                product,
                INTENT_VERSION,
                intent_digest(&operation, job, &[], 0),
            ),
            // A removal waits for nothing: the listing it addresses already
            // exists, which is what `Binding::Bound` means.
            requires_bound_on: None,
            operation,
        };
        let created = jobs
            .create_with_request_key(
                context.org,
                // Derived rather than taken from a header: a repeated delete
                // must replay the first removal job rather than enqueue a
                // second write against a listing the first one removed.
                tam_storage::job_request_key(product.0, &format!("{DELETE_LEG}:{inventory:?}")),
                &NewJob {
                    job,
                    inventory,
                    stamp: Stamp {
                        at: now,
                        actor: Actor::Person(context.user),
                    },
                },
                &[item],
            )
            .await
            .map_err(|error| storage_fault(&state, &error))?;
        removals.push(RemovalView {
            inventory,
            job: created.job,
            mapping: record.mapping.id,
        });
    }

    let deleted = ProductRepo::new(state.pool.clone())
        .soft_delete(context.org, product, now)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if !deleted && removals.is_empty() {
        return Err(missing("no such product"));
    }
    Ok(Json(DeletedProductView {
        product,
        removals,
        left_live: unelected,
    }))
}

// ------------------------------------------------------------------- mount

/// The upload route's own body ceiling, which is the one route that carries
/// resource bytes. Every other route keeps axum's default, which coincides
/// with `tam_limits::http::REQUEST_BODY_BYTES_MAX`.
pub fn upload_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(
        usize::try_from(tam_limits::http::UPLOAD_BODY_BYTES_MAX).unwrap_or(usize::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        hex_encode, parse_hash, required_fields_answered, trigger_kind_of, uncaptured_edits,
        ElectionInput, FileHandle, RightsInput,
    };
    use tam_domain::equivalence::ElectionTriggerKind;
    use tam_domain::{
        Binding, FieldPolicies, FieldPolicy, Mapping, PublishMode, TermKind, Verification,
    };
    use tam_marketplace::{RemoteLifecycle, RemoteListingId};
    use tam_storage::MappingRecord;
    use tam_types::{
        FileRole, InventoryId, MappingId, OrgId, PriceIntent, PriceRule, ProductId, Timestamp, Uuid,
    };

    fn licence_answer(inventory: InventoryId) -> ElectionInput {
        ElectionInput {
            inventory,
            axis: TermKind::Licence,
            trigger: "supply".to_owned(),
            trigger_key: Some("free".to_owned()),
            answers: vec![super::AnswerInput {
                segments: vec!["CC-BY".to_owned()],
                native_id: Some("CC-BY".to_owned()),
            }],
        }
    }

    fn bound_mapping(inventory: InventoryId, lifecycle: RemoteLifecycle) -> MappingRecord {
        MappingRecord {
            mapping: Mapping {
                id: MappingId(Uuid([0x31; 16])),
                org: OrgId(Uuid([0xAA; 16])),
                product: ProductId(Uuid([0x01; 16])),
                inventory,
                binding: Binding::Bound {
                    id: match inventory {
                        InventoryId::Tpt => RemoteListingId::Tpt { product_id: 77 },
                        InventoryId::Etsy => RemoteListingId::Etsy { listing_id: 77 },
                        InventoryId::TesGb | InventoryId::TesUs | InventoryId::TesNz => {
                            RemoteListingId::Tes {
                                url: "https://www.tes.com/teaching-resource/x-77".to_owned(),
                            }
                        }
                    },
                    first_seen: Timestamp(1),
                    verified: Verification::Clean { at: Timestamp(1) },
                },
                policies: FieldPolicies {
                    title: FieldPolicy::Managed,
                    description: FieldPolicy::Managed,
                    price: FieldPolicy::Managed,
                    taxonomy: FieldPolicy::Managed,
                    grades: FieldPolicy::Managed,
                    files: FieldPolicy::Managed,
                },
                price_rule: PriceRule::Explicit(PriceIntent::Free),
                publish: PublishMode::DryRun,
                lifecycle,
            },
            normaliser_version: 0,
            created_at: Timestamp(1),
            updated_at: Timestamp(1),
        }
    }

    #[test]
    fn a_hash_round_trips_through_its_hex_spelling() {
        let hash = tam_types::ContentHash([0xAB; 32]);
        let hex = hex_encode(&hash.0);
        assert_eq!(hex.len(), 64, "a blake3 digest is 64 hex characters");
        assert_eq!(parse_hash(&hex), Some(hash), "the spelling round-trips");
        assert_eq!(parse_hash("abc"), None, "a short digest is refused");
        assert_eq!(
            parse_hash(&"zz".repeat(32)),
            None,
            "a non-hex digest is refused rather than silently zeroed"
        );
    }

    #[test]
    fn a_handle_resolves_to_a_payload_file_with_its_own_kind() {
        let handle = FileHandle {
            hash: hex_encode(&[0x11; 32]),
            kind: "pdf".to_owned(),
            byte_len: 9,
        };
        let file = handle
            .resolve(FileRole::Payload, tam_types::Timestamp(9))
            .expect("a well-formed handle resolves");
        assert_eq!(
            (file.role, file.kind, file.byte_len),
            (FileRole::Payload, tam_types::FileKind::Pdf, 9),
            "the handle's own kind and length reach the catalogue row"
        );
        let refused = FileHandle {
            kind: "exe".to_owned(),
            ..handle
        }
        .resolve(FileRole::Payload, tam_types::Timestamp(9));
        assert!(
            refused.is_err(),
            "a kind this server does not store is refused rather than coerced"
        );
    }

    #[test]
    fn the_tes_licence_is_the_only_required_field_and_either_answer_satisfies_it() {
        let tes = [InventoryId::TesGb];
        assert!(
            required_fields_answered(&tes, None, &[]).is_err(),
            "Tes declares its licence required, so a create carrying neither is refused"
        );
        assert!(
            required_fields_answered(
                &tes,
                Some(&RightsInput {
                    inventory: InventoryId::TesGb,
                    segments: vec!["CC-BY".to_owned()],
                    native_id: Some("CC-BY".to_owned()),
                }),
                &[]
            )
            .is_ok(),
            "a stated rights grant is a licence"
        );
        assert!(
            required_fields_answered(&tes, None, &[licence_answer(InventoryId::TesGb)]).is_ok(),
            "so is an already-answered licence election for that inventory"
        );
        assert!(
            required_fields_answered(&tes, None, &[licence_answer(InventoryId::TesUs)]).is_err(),
            "an answer for a different inventory does not satisfy this one"
        );
    }

    #[test]
    fn tpt_declares_nothing_required_so_a_bare_create_is_admitted() {
        assert!(
            required_fields_answered(&[InventoryId::Tpt], None, &[]).is_ok(),
            "no TPT capture contains a refused write, so the registry declares nothing required \
             and the server's own refusals speak instead"
        );
    }

    #[test]
    fn a_live_tes_listing_refuses_the_edit_and_a_live_tpt_listing_does_not() {
        let tes = uncaptured_edits(&[bound_mapping(
            InventoryId::TesNz,
            RemoteLifecycle::Live {
                since: Timestamp(2),
            },
        )]);
        assert_eq!(
            tes,
            vec![(InventoryId::TesNz, "tes.edit_published")],
            "Tes serves neither live-to-live nor live-to-draft, so the edit is refused by name"
        );
        assert!(
            uncaptured_edits(&[bound_mapping(
                InventoryId::Tpt,
                RemoteLifecycle::Live {
                    since: Timestamp(2)
                }
            )])
            .is_empty(),
            "TPT serves all four transitions"
        );
        assert!(
            uncaptured_edits(&[bound_mapping(InventoryId::TesNz, RemoteLifecycle::Draft)])
                .is_empty(),
            "a Tes draft is editable; only a live one is not"
        );
    }

    #[test]
    fn the_trigger_vocabulary_is_the_one_the_ledger_stores() {
        for kind in ElectionTriggerKind::ALL {
            assert_eq!(
                trigger_kind_of(kind.as_str()),
                Some(kind),
                "the wire spelling is the column's spelling"
            );
        }
        assert_eq!(
            trigger_kind_of("licence"),
            None,
            "an axis name is not a trigger kind"
        );
    }
}
