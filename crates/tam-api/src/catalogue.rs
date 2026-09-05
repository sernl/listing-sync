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
    Actor, CanonicalTermId, ContentHash, CopyFormat, FileBytes, FileId, FileRole, InventoryId,
    JobId, ListingCopy, MappingId, Money, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile,
    ProductId, ScanOutcome, Stamp, Timestamp, Title, Uuid,
};

use tam_authoring::refusal_of;
use tam_domain::product::ProductName;

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::product::{record_of, verdict, DraftHead, TptBaseInput};
use crate::quota::{quota_for, QuotaKind};
use crate::resources::{file_view, kind_from_str, kind_str, FileView, MappingHeadView};
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
    /// What the seller called the file they chose.
    ///
    /// The client's word and never ours: the upload route reads the request
    /// body as raw bytes, which carry no filename, so the name comes back
    /// beside the handle on the create or the file write rather than out of
    /// anything the pipeline probed. Absent is a handle nobody named — the
    /// generated cover, an entry of an exploded archive, an older client —
    /// and the console renders the absence rather than inventing a name.
    #[serde(default)]
    pub name: Option<String>,
}

/// The longest name `product_file_name_length` admits.
const FILE_NAME_MAX: usize = 255;

impl FileHandle {
    fn of(file: &tam_pipeline::pipeline::IngestedFile) -> Self {
        Self {
            name: None,
            hash: hex_encode(&file.hash.0),
            kind: kind_str(file.kind).to_owned(),
            byte_len: file.byte_len,
        }
    }

    /// The name this handle carries, refused where it would not fit the
    /// column. Checked here rather than at each caller so the create, the add
    /// and the replace cannot come to different conclusions about one field.
    fn checked_name(&self) -> Result<Option<&str>, APIError> {
        let Some(name) = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        else {
            return Ok(None);
        };
        if name.chars().count() > FILE_NAME_MAX {
            return Err(validation(
                "a file's name is longer than the 255 characters a listing can carry",
            ));
        }
        Ok(Some(name))
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
            // Held: a handle names bytes this tenant has already uploaded, and
            // the create refuses one whose hash we do not hold, so there is no
            // route from this constructor to a sourced file.
            bytes: FileBytes::Held {
                hash,
                byte_len: self.byte_len,
                // The pipeline scanned the bytes before they were stored, and
                // a handle exists only because that scan passed.
                scan: ScanOutcome::Clean { at: now },
            },
        })
    }
}

/// Resolves one handle and records what the seller called it, keyed by the
/// identity the resolve just minted.
///
/// The two steps are one function because the key is that identity: the
/// resolve mints a fresh `FileId`, so a caller that resolved first and named
/// afterwards would have to keep the handle and the file paired by hand across
/// four collections, and the first one it mispaired would put a worksheet's
/// name on an answer key.
fn resolve_named(
    handle: &FileHandle,
    role: FileRole,
    now: Timestamp,
    names: &mut std::collections::HashMap<FileId, String>,
) -> Result<ProductFile, APIError> {
    let file = handle.resolve(role, now)?;
    if let Some(name) = handle.checked_name()? {
        names.insert(file.id, name.to_owned());
    }
    Ok(file)
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

pub(crate) fn parse_hash(raw: &str) -> Option<ContentHash> {
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

/// Refuses a title the domain will not hold, in the seller's own words.
///
/// Through `ProductName::new` rather than an emptiness check of its own, so
/// this route and the operator import agree: the import already builds the
/// same type, and a title 81 UTF-16 units long was accepted here and refused
/// there. The cap is TPT's own, measured from its form, and it was reachable
/// from this route only when a TPT-base block happened to be present — so a
/// create without one could store a title no marketplace would take.
///
/// The sentence is `tam_authoring`'s, rendered by the same `refusal_of` the
/// check endpoint renders every other authoring refusal with, so the two
/// surfaces cannot word one rule differently.
fn checked_title(raw: &str) -> Result<ProductName, APIError> {
    ProductName::new(raw).map_err(|error| {
        let view = refusal_of(&error);
        APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new(&view.message)
                .code(APIErrorCode::RequiredFieldMissing)
                .kind(APIErrorKind::Validation),
        )
    })
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
///
/// The refusal names the marketplace and the field for every one that is
/// missing, and names the field twice — once as the wire spells it, so a
/// client can anchor to the control, and once as the platform's own form
/// heads it, so the sentence a seller reads is in their words. A client that
/// renders the bare message and discards the detail tells a seller a field is
/// missing without saying which, which is the whole reason the detail is a
/// contract rather than a convenience.
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
                    // The words the platform's own form heads the control
                    // with, so the refusal a seller reads names the field
                    // they would recognise rather than the wire name beside
                    // it. The wire name where no capture recorded words,
                    // which is the same fallback the vocabulary view makes.
                    "label": native.label.unwrap_or(native.name),
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
        // The destination is not knowable from these five fields; the create
        // path overrides it from its own inventory list, which is the only
        // place that holds one.
        for_marketplace: false,
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
    checked_title(&body.title)?;
    // Named before the general refusal below, because the two are different
    // situations and only this one is about something the seller just chose. A
    // create naming a marketplace and carrying no file cannot be listed there
    // whatever else is true of it, and saying so by name is what lets the form
    // answer "add your file first" rather than restating the rule for a draft
    // that was never going anywhere.
    if !body.inventories.is_empty() && body.payload.is_empty() {
        return Err(coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            "a listing on a marketplace needs a file buyers can download; upload it first",
            APIErrorCode::PayloadMissing,
        ));
    }
    // No payload at all is a resource kept on Teachouse (D32), so the general
    // refusal that used to stand here is gone and the marketplace case above is
    // what replaced it. The deferred trigger now raises only for a product a
    // mapping names, so the two say the same thing.
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
            // The destination is the create body's own, not the block's: a
            // draft bound for a marketplace needs a file and one kept here
            // does not (D32), and only this body knows which it is.
            let draft = base.into_draft(DraftHead {
                for_marketplace: !body.inventories.is_empty(),
                ..draft_head(&body.title, &body.body, price, &body.payload, &body.grades)
            });
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
    //
    // The sidecar's thumbnail hashes are checked here too, and were not before.
    // They travel as bare digests rather than as `FileHandle`s, so they missed
    // the chain below and reached `product_tpt_base` unverified: a create could
    // name four digests this organisation never uploaded and the row would
    // record them, leaving the TPT write pointing at bytes that do not exist.
    let mut claimed: Vec<ContentHash> = body
        .payload
        .iter()
        .chain(body.cover.iter())
        .chain(body.previews.iter())
        .map(|handle| {
            parse_hash(&handle.hash)
                .ok_or_else(|| validation("a file handle's hash is not a 64-character hex digest"))
        })
        .collect::<Result<Vec<_>, APIError>>()?;
    // The digest is already well-formed by the time this runs — `record_of`
    // above reads the same strings through `UploadRef::new` and refuses a
    // malformed one as an authoring refusal — so this arm exists to keep the
    // conversion total rather than to catch anything a caller can reach today.
    // What it does catch is the held-bytes question, which nothing asked.
    for hash in body
        .tpt_base
        .iter()
        .flat_map(|base| base.thumbnail_hashes.iter())
    {
        claimed
            .push(parse_hash(hash).ok_or_else(|| {
                validation("a thumbnail's hash is not a 64-character hex digest")
            })?);
    }
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

    let mut names = std::collections::HashMap::new();
    // Non-empty by construction wherever it exists: the option carries the
    // absence and `PayloadSet` carries nothing else, so there is no third state
    // in which a set exists and holds no file.
    let payload = match body.payload.split_first() {
        None => None,
        Some((head, rest)) => Some(PayloadSet::new(
            resolve_named(head, FileRole::Payload, now, &mut names)?,
            rest.iter()
                .map(|handle| resolve_named(handle, FileRole::Payload, now, &mut names))
                .collect::<Result<Vec<_>, APIError>>()?,
        )),
    };
    // The cover is generated from the first payload's bytes rather than
    // chosen, so no name is recorded for it even where a client sends one.
    let cover = body
        .cover
        .as_ref()
        .map(|handle| handle.resolve(FileRole::Cover, now))
        .transpose()?;
    let previews = body
        .previews
        .iter()
        .map(|handle| resolve_named(handle, FileRole::Preview, now, &mut names))
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
        .insert_named(
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
            &names,
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

    // The other half of D32, and the half that is not vacuous: a resource kept
    // on Teachouse may carry no file, so the moment it is pointed at a
    // marketplace is the moment one becomes necessary. Refused here rather than
    // at the write, because a mapping is what a publish lowers and a seller who
    // learns this from a failed job learns it far too late.
    if !stored.product.has_payload() {
        return Err(coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            "a listing on a marketplace needs a file buyers can download; upload it first",
            APIErrorCode::PayloadMissing,
        ));
    }

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
    // The same rule as the create, on the one route that can change a title
    // after it. An edit carries a title only when it changes one, so the check
    // runs on what was sent rather than on an absent field read as blank.
    let title = body
        .title
        .as_deref()
        .map(checked_title)
        .transpose()?
        .map(|name| Title(name.as_str().to_owned()));
    let edit = ProductEdit {
        title,
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
            // False for the same reason `payload_hash` is None: an edit carries
            // neither, and holding it to the marketplace rule would refuse
            // every edit of a listing that has a file, for not resending it.
            for_marketplace: false,
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

// ------------------------------------------------------------------- files

/// One file added to a product that already exists.
///
/// The role is the client's to state, because a preview and a payload arrive
/// by the same upload and only the seller knows which they meant. A cover is
/// accepted for the product that has none, and refused for the product that
/// has one, which is `product_file_one_cover` said in a sentence.
#[derive(Debug, Clone, Deserialize)]
pub struct AddFileBody {
    /// `payload`, `cover` or `preview`, spelled as the product view spells it.
    pub role: String,
    pub handle: FileHandle,
}

/// The bytes one file is to be swapped for.
///
/// One field, and deliberately no others. No role, because a replacement keeps
/// the role of the file it replaces. And no cover: the thumbnail this write
/// redraws is rendered here from these bytes rather than named by the caller,
/// so there is no handle for a client to point at a file of its choosing. A
/// field that cannot be sent is one that cannot be abused, which is why this
/// closes the hole rather than checking it.
#[derive(Debug, Clone, Deserialize)]
pub struct ReplaceFileBody {
    pub handle: FileHandle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddedFileView {
    pub product: ProductId,
    pub file: FileView,
    /// The platforms this change reaches on the next send, exactly as
    /// [`PatchedProductView`] reports them. Nothing here contacts a
    /// marketplace, so the copy already on one stands until that send.
    pub reaches: Vec<InventoryId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplacedFileView {
    pub product: ProductId,
    /// The file that stopped being this resource's. Reported because the
    /// replacement is a new row with a new identifier, and a client keying on
    /// the old one would otherwise go on rendering it.
    pub removed: Uuid,
    pub file: FileView,
    /// The cover this replacement redrew, absent where it did not.
    ///
    /// Reported rather than inferred from having offered one: the two
    /// conditions that decide it are the server's, so a client that assumed
    /// would tell the seller their thumbnail changed when it had not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover: Option<FileView>,
    pub reaches: Vec<InventoryId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovedFileView {
    pub product: ProductId,
    pub file: Uuid,
    /// What became of the thumbnail. Three states rather than an optional
    /// file, because a removal can leave it alone, redraw it, or take it away,
    /// and the last is not the absence of the second.
    pub thumbnail: ThumbnailView,
    pub reaches: Vec<InventoryId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ThumbnailView {
    /// The removed file was not the one it was drawn from.
    Untouched,
    /// Redrawn from the file that became the first one.
    Redrawn { file: FileView },
    /// Taken away, because the resource has no file left to draw one from. A
    /// picture of a file the resource does not hold is worse than no picture.
    Retired,
}

fn role_from_str(raw: &str) -> Option<FileRole> {
    match raw {
        "payload" => Some(FileRole::Payload),
        "cover" => Some(FileRole::Cover),
        "preview" => Some(FileRole::Preview),
        _ => None,
    }
}

fn parse_file_id(raw: &str) -> Result<FileId, APIError> {
    uuid::Uuid::parse_str(raw)
        .map(|parsed| FileId(Uuid(*parsed.as_bytes())))
        .map_err(|_| missing("no such file"))
}

/// The sentence each storage refusal reaches the seller as.
///
/// `LastPayload` carries [`APIErrorCode::PayloadMissing`] rather than a code
/// of its own: it is the same invariant a payload-less create is refused by,
/// stated at the other end of the resource's life.
fn file_refusal(state: &AppState, refusal: tam_storage::FileRefusal) -> APIError {
    match refusal {
        tam_storage::FileRefusal::NoProduct => missing("no such product"),
        tam_storage::FileRefusal::NoFile => missing("no such file"),
        tam_storage::FileRefusal::LastPayload => coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            "a resource keeps at least one file, so this one cannot be removed; \
             replace it, or add another first",
            APIErrorCode::PayloadMissing,
        ),
        // Unreachable through this API: no route asks the repository to add a
        // cover any more, because a client may not name that role and the
        // redraw replaces the row rather than adding to it. Kept as a fault
        // rather than deleted, because `add_file` is a public method of the
        // repository and a future caller could still reach the guard — and
        // reaching it from here would mean our own routes had disagreed with
        // themselves, which is ours to see rather than a seller's to read.
        tam_storage::FileRefusal::CoverExists => {
            state.internal("a file route asked the catalogue to add a second thumbnail")
        }
    }
}

/// Which marketplaces this change reaches on the next send, and the refusal
/// where a live listing cannot be revised at all.
///
/// The same gate the edit passes through, for the same reason: a file change
/// reaches a marketplace as a revise, so a listing whose revise no capture
/// supports cannot have its file changed through us either, and the seller is
/// told before the catalogue is written rather than after an item settles.
async fn refuse_uncaptured(
    state: &AppState,
    org: OrgId,
    product: ProductId,
) -> Result<(), APIError> {
    let mappings = MappingRepo::new(state.pool.clone())
        .list_for_product(org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let refused = uncaptured_edits(&mappings);
    if !refused.is_empty() {
        return Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new(
                "this listing is live on a platform whose edit-published transition is \
                 uncaptured, so its files cannot be changed",
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
    Ok(())
}

/// Which marketplaces this change reaches on the next send, read after the
/// write rather than before it.
///
/// The two halves are separate calls on purpose. The refusal has to run before
/// the write, because its whole point is to decline before the catalogue moves;
/// the reported reach has to be read after, because a mapping bound or unbound
/// while the write held the product's row would otherwise be described by a
/// value taken before it. The response then says where the change actually
/// landed rather than where it was going to.
async fn reaches_after(
    state: &AppState,
    org: OrgId,
    product: ProductId,
) -> Result<Vec<InventoryId>, APIError> {
    Ok(MappingRepo::new(state.pool.clone())
        .list_for_product(org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .iter()
        .map(|record| record.mapping.inventory)
        .collect())
}

/// Refuses a handle naming bytes this tenant has never uploaded, which is the
/// create's own check applied to the one handle a file change carries.
async fn held(state: &AppState, org: OrgId, handle: &FileHandle) -> Result<(), APIError> {
    let claimed = parse_hash(&handle.hash)
        .ok_or_else(|| validation("a file handle's hash is not a 64-character hex digest"))?;
    let known = ProductRepo::new(state.pool.clone())
        .stored_hashes(org, &[claimed])
        .await
        .map_err(|error| storage_fault(state, &error))?;
    if known.is_empty() {
        return Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new("a file handle names bytes this organisation has not uploaded")
                .code(APIErrorCode::UploadRejected)
                .kind(APIErrorKind::Validation)
                .detail(serde_json::json!({ "hashes": [hex_encode(&claimed.0)] })),
        ));
    }
    Ok(())
}

/// Refused because a cover is ours to draw and never a client's to name.
///
/// The console reads a cover back as an image, so a row whose role is `cover`
/// is a row whose bytes a browser will fetch. Letting a client point that role
/// at any hash the organisation holds therefore turns this route into a way to
/// make a sellable payload file browser-readable, and the only thing that ever
/// stopped it was that our own client did not ask. `ingest` draws every cover
/// from the first payload's bytes and nothing else writes one, which is what
/// this refusal states in the code rather than only in the design.
fn cover_not_yours() -> APIError {
    validation(
        "the thumbnail is drawn from the resource's first file and is not one to upload directly",
    )
}

/// What a replacement needs to know about the product before it writes: which
/// row is the cover, and which is the payload file the cover was drawn from.
///
/// One read rather than two, and a read rather than a check inside the
/// repository, because `product.rs` is not this slice's to edit today. A row's
/// role never changes and its position never moves, so nothing here can go
/// stale between the read and the write; a row that disappears in between is
/// refused by the repository as no such file, and the repository re-checks the
/// redraw condition under its own lock regardless.
struct CoverFacts {
    cover: Option<FileId>,
    first_payload: Option<FileId>,
    /// The payload file that becomes the first one when the first is removed,
    /// and therefore the one a redrawn thumbnail is rendered from. Absent
    /// where the product has only one, which is a removal the repository
    /// refuses anyway.
    next_payload: Option<ProductFile>,
}

async fn cover_facts(
    state: &AppState,
    org: OrgId,
    product: ProductId,
) -> Result<CoverFacts, APIError> {
    let record = ProductRepo::new(state.pool.clone())
        .get(org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("no such product"))?;
    let mut payloads = record.product.payload_files();
    let first_payload = payloads.next().map(|file| file.id);
    let next_payload = payloads.next().cloned();
    Ok(CoverFacts {
        cover: record.product.cover.as_ref().map(|cover| cover.id),
        first_payload,
        next_payload,
    })
}

/// Draws the thumbnail for a replacement, from the replacement's own bytes.
///
/// This is the whole of the fix for the third door: the cover handle used to
/// arrive in the request body, so a client could name any hash the
/// organisation held and have it written behind the one role the console
/// renders as an image. Nothing the caller sends reaches this function — it
/// reads the bytes back from the object store by the hash it just verified, it
/// renders them through the same `tam_pipeline::render::cover` the upload
/// used, and it stores the result under this tenant's own sink.
///
/// The read-back is a real cost and is stated rather than hidden: it holds the
/// replacement in memory exactly as the upload that produced it did, bounded
/// by the same `UPLOAD_BODY_BYTES_MAX`.
async fn redraw_cover(
    state: &AppState,
    org: OrgId,
    file: &ProductFile,
    now: Timestamp,
) -> Result<tam_storage::FileReplacement, APIError> {
    let blobs = state.blobs.clone().ok_or_else(|| {
        APIError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            APIErrorEntry::new(
                "this deployment holds no key-encryption key or object-store root, so the                  thumbnail cannot be redrawn and the file was not replaced",
            )
            .code(APIErrorCode::BlobStoreUnavailable)
            .kind(APIErrorKind::Internal),
        )
    })?;
    let FileBytes::Held { hash, .. } = file.bytes else {
        return Err(state.internal("a replacement resolved to bytes this server does not hold"));
    };
    let repo = BlobRepo::new(
        state.pool.clone(),
        LocalObjectStore::new(blobs.root.clone()),
        blobs.kek.clone(),
    );
    let bytes = repo
        .get(org, hash)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let rendered = tam_pipeline::render::cover(file.kind, &bytes)
        .map_err(|error| state.internal(&format!("the thumbnail could not be drawn: {error}")))?;
    let sink = TenantBlobSink {
        repo: &repo,
        org,
        at: now,
    };
    let drawn = tam_pipeline::pipeline::BlobSink::store(&sink, rendered.png.clone())
        .await
        .map_err(|error| state.internal(&error))?;
    Ok(tam_storage::FileReplacement {
        id: FileId(fresh_uuid()),
        kind: tam_types::FileKind::Image,
        bytes: FileBytes::Held {
            hash: drawn,
            byte_len: rendered.png.len() as u64,
            // Rendered here from bytes this server already scanned at upload,
            // so the verdict is ours to state rather than a client's.
            scan: ScanOutcome::Clean { at: now },
        },
        name: None,
    })
}

pub(crate) async fn add_file(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, product)): Path<(String, String)>,
    Json(body): Json<AddFileBody>,
) -> Result<(StatusCode, Json<AddedFileView>), APIError> {
    let product = parse_product_id(&product)?;
    let role = role_from_str(&body.role)
        .ok_or_else(|| validation("a file names a role this server does not store"))?;
    if role == FileRole::Cover {
        return Err(cover_not_yours());
    }
    let now = (state.wall)();
    refuse_uncaptured(&state, context.org, product).await?;
    held(&state, context.org, &body.handle).await?;
    let file = body.handle.resolve(role, now)?;
    let name = body.handle.checked_name()?;
    ProductRepo::new(state.pool.clone())
        .add_file(context.org, product, (&file, name), now)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .map_err(|refusal| file_refusal(&state, refusal))?;
    let reaches = reaches_after(&state, context.org, product).await?;
    Ok((
        StatusCode::CREATED,
        Json(AddedFileView {
            product,
            file: file_view(&file, name),
            reaches,
        }),
    ))
}

pub(crate) async fn replace_file(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, product, file)): Path<(String, String, String)>,
    Json(body): Json<ReplaceFileBody>,
) -> Result<Json<ReplacedFileView>, APIError> {
    let product = parse_product_id(&product)?;
    let replaced = parse_file_id(&file)?;
    let now = (state.wall)();
    refuse_uncaptured(&state, context.org, product).await?;
    let facts = cover_facts(&state, context.org, product).await?;
    // The cover row is not a target. Replacing it would let a client put any
    // hash the organisation holds behind the one role the console renders as
    // an image, which is the same hole as naming the role on an add.
    if facts.cover == Some(replaced) {
        return Err(cover_not_yours());
    }
    held(&state, context.org, &body.handle).await?;
    // Resolved into the payload slot only to reach the parsed kind and bytes;
    // the role the row keeps is the stored one, which `replace_file` reads
    // under its own lock and `FileReplacement` deliberately cannot state.
    let parsed = body.handle.resolve(FileRole::Payload, now)?;
    // The thumbnail is drawn from the first payload file, so replacing that
    // file redraws it. Decided here only to avoid rendering one nobody will
    // use; the repository re-checks the same condition under its own lock, so
    // a cover drawn against a product that moved underneath is discarded
    // rather than written.
    let cover = if facts.first_payload == Some(replaced) && facts.cover.is_some() {
        Some(redraw_cover(&state, context.org, &parsed, now).await?)
    } else {
        None
    };
    let written = ProductRepo::new(state.pool.clone())
        .replace_file(
            context.org,
            tam_storage::FileTarget {
                product,
                file: replaced,
            },
            &tam_storage::FileSwap {
                file: tam_storage::FileReplacement {
                    id: parsed.id,
                    kind: parsed.kind,
                    bytes: parsed.bytes,
                    name: body.handle.checked_name()?.map(str::to_owned),
                },
                cover,
            },
            now,
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .map_err(|refusal| file_refusal(&state, refusal))?;
    let reaches = reaches_after(&state, context.org, product).await?;
    Ok(Json(ReplacedFileView {
        product,
        removed: replaced.0,
        file: file_view(&written.file, body.handle.checked_name()?),
        // No name: the cover was drawn from the new bytes, not chosen.
        cover: written.cover.as_ref().map(|drawn| file_view(drawn, None)),
        reaches,
    }))
}

pub(crate) async fn remove_file(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, product, file)): Path<(String, String, String)>,
) -> Result<Json<RemovedFileView>, APIError> {
    let product = parse_product_id(&product)?;
    let removed = parse_file_id(&file)?;
    let now = (state.wall)();
    refuse_uncaptured(&state, context.org, product).await?;
    let facts = cover_facts(&state, context.org, product).await?;
    // The thumbnail is drawn from the first payload file, so removing that
    // file leaves it depicting a file the resource no longer holds. It is
    // redrawn from the file that becomes the first one — rendered here from
    // that file's own bytes, never from anything the caller sends, for the
    // same reason a replacement's is.
    let redrawn = match (&facts.next_payload, facts.first_payload, facts.cover) {
        (Some(next), Some(first), Some(_)) if first == removed => {
            Some(redraw_cover(&state, context.org, next, now).await?)
        }
        _ => None,
    };
    let cover = ProductRepo::new(state.pool.clone())
        .remove_file(
            context.org,
            tam_storage::FileTarget {
                product,
                file: removed,
            },
            redrawn.as_ref(),
            now,
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .map_err(|refusal| file_refusal(&state, refusal))?;
    let reaches = reaches_after(&state, context.org, product).await?;
    Ok(Json(RemovedFileView {
        product,
        file: removed.0,
        thumbnail: match &cover {
            tam_storage::ThumbnailChange::Untouched => ThumbnailView::Untouched,
            tam_storage::ThumbnailChange::Redrawn(drawn) => ThumbnailView::Redrawn {
                file: file_view(drawn, None),
            },
            tam_storage::ThumbnailChange::Retired => ThumbnailView::Retired,
        },
        reaches,
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
        ElectionInput, FileHandle, RightsInput, FILE_NAME_MAX,
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
            name: Some("  worksheet.pdf  ".to_owned()),
        };
        assert_eq!(
            handle.checked_name().expect("a short name is admitted"),
            Some("worksheet.pdf"),
            "the surrounding space a file picker leaves is not part of the name"
        );
        assert_eq!(
            FileHandle {
                name: Some("   ".to_owned()),
                ..handle.clone()
            }
            .checked_name()
            .expect("a blank name is admitted as no name"),
            None,
            "a name that is only space is no name, not an empty one"
        );
        assert!(
            FileHandle {
                name: Some("x".repeat(FILE_NAME_MAX + 1)),
                ..handle.clone()
            }
            .checked_name()
            .is_err(),
            "a name longer than the column is refused here rather than at the insert"
        );
        let file = handle
            .resolve(FileRole::Payload, tam_types::Timestamp(9))
            .expect("a well-formed handle resolves");
        assert_eq!(
            (file.role, file.kind, file.bytes.byte_len()),
            (FileRole::Payload, tam_types::FileKind::Pdf, 9),
            "the handle's own kind and length reach the catalogue row"
        );
        let refused = FileHandle {
            kind: "exe".to_owned(),
            ..handle.clone()
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
