//! The catalogue, connections and reconciliation surfaces: what M1i's
//! client renders and the founder's drain workflow drives. Connection
//! revocation is a control-plane write on the connection row: the vault it
//! used to tombstone through the session broker went with D1, along with
//! every server-side seller session, so there is no socket to be without.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_domain::equivalence::{
    best_fit, resolution_for, Candidate, ElectionAnswer, ElectionRule, ElectionRuleError,
    ElectionTrigger, ElectionTriggerKind, Mode, NewElectionRule, NewProjectionOverride,
    OverrideKind, ProjectionOverride, ProjectionOverrideError, Ranked, Suggestion,
};
use tam_domain::registry::listing_url::{listing_url, parse_listing_url, UrlRefusal};
use tam_domain::registry::{registry, AxisBinding, Cardinality, CountCap};
use tam_domain::{
    CanonicalProduct, CanonicalTerm, Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId,
    VocabularyPath,
};
use tam_storage::{
    BlobError, BlobRepo, ConnectionFactsRepo, ConnectionRepo, DrainStats, ElectionRepo,
    LabelRename, LabelRepo, LedgerCursor, MappingRepo, NewAnswer, OpenElection, OverrideRepo,
    PastedBind, ProductRepo, StorageError, TaxonomyRepo,
};
use tam_taxonomy::check_native_ids;
use tam_taxonomy::listing::projection_vocabularies;
use tam_taxonomy::project::{ingest_grades, project_terms};
use tam_types::{
    Actor, CanonicalTermId, ConnectionId, ConnectionStatus, CopyFormat, InventoryId, MappingId,
    Marketplace, OrgId, PriceIntent, ProductId, ScanOutcome, Stamp, Timestamp, TransportClass,
    Uuid,
};

use crate::catalogue::parse_hash;
use crate::entitlement::{quota_refusal, QuotaKind};
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::jobs::{decode_cursor, encode_cursor};
use crate::product::{StandardInput, TptBaseInput};
use crate::{AppState, OrgContext};

fn storage_fault(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    crate::jobs::storage_fault(state, error)
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
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

fn parse_id(raw: &str) -> Result<Uuid, APIError> {
    uuid::Uuid::parse_str(raw)
        .map(|parsed| Uuid(*parsed.as_bytes()))
        .map_err(|_| validation("the identifier is not a UUID"))
}

const PAGE_LIMIT_DEFAULT: i64 = 50;
const PAGE_LIMIT_MAX: i64 = 200;

/// How deep a path a client names may be, and how long one segment of it may
/// run.
///
/// Measured against the committed vocabularies rather than picked: the deepest
/// real path is two segments, a Tes subject and its topic, and the longest
/// label is 69 characters, in TPT's licence list. Four times that depth and
/// roughly three times that length leaves room for a vocabulary that grows,
/// without leaving an array a stranger fills at their own discretion. Both are
/// limits a founder may replace.
const PATH_SEGMENTS_MAX: usize = 8;
const PATH_SEGMENT_CHARS_MAX: usize = 200;

/// The one shape check every path a client names passes.
///
/// Shared by the two handlers that write one: the reconciliation queue's
/// resolution and the seller's own override. Both take a path from a request
/// body and make it durable, so a bound on one and not the other is a bound
/// with a way around it.
fn check_path(segments: &[String]) -> Result<(), APIError> {
    if segments.is_empty() {
        return Err(validation("an edge needs at least one path segment"));
    }
    if segments.len() > PATH_SEGMENTS_MAX {
        return Err(validation(
            "a path is at most eight segments deep; the deepest any marketplace publishes is two",
        ));
    }
    if let Some(long) = segments
        .iter()
        .find(|segment| segment.chars().count() > PATH_SEGMENT_CHARS_MAX)
    {
        return Err(validation(&format!(
            "a path segment is at most two hundred characters, and one here is {}",
            long.chars().count()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------- catalogue

#[derive(Debug, Serialize, Deserialize)]
pub struct ProductsPage {
    pub products: Vec<ProductHead>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProductHead {
    pub id: ProductId,
    pub title: String,
    pub price: PriceIntent,
    /// Where this resource's cover can be fetched, or absent where it has no
    /// stored cover.
    ///
    /// A URL rather than a flag, because the client renders it directly and a
    /// client that had to compose the path would be a second place the route
    /// is written down. Absent rather than always present so a row with no
    /// cover draws its placeholder instead of asking for bytes that are not
    /// there.
    pub cover: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// The catalogue page, and the one narrowing it admits.
///
/// `label` is the seller's own vocabulary rather than ours, so it is matched
/// as text; an unknown label is an empty page rather than a refusal, because a
/// label the seller has just removed from every item stops existing and a
/// bookmarked filter naming it should say "nothing here", not "you are wrong".
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CataloguePageParams {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub label: Option<String>,
}

pub(crate) async fn list_products(
    State(state): State<AppState>,
    context: OrgContext,
    Path((version,)): Path<(String,)>,
    Query(params): Query<CataloguePageParams>,
) -> Result<Json<ProductsPage>, APIError> {
    let cursor = match params.cursor.as_deref() {
        None => None,
        Some(raw) => Some(
            decode_cursor(raw)
                .ok_or_else(|| validation("the cursor is not one this server issued"))?,
        ),
    };
    let limit = params
        .limit
        .unwrap_or(PAGE_LIMIT_DEFAULT)
        .clamp(1, PAGE_LIMIT_MAX);
    let rows = ProductRepo::new(state.pool.clone())
        .list_page(
            context.org,
            cursor,
            limit,
            params
                .label
                .as_deref()
                .map(str::trim)
                .filter(|label| !label.is_empty()),
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let next_cursor = (i64::try_from(rows.len()).unwrap_or(i64::MAX) == limit)
        .then(|| {
            rows.last().map(|last| {
                encode_cursor(&LedgerCursor {
                    created_at: last.created_at,
                    id: last.id.0,
                })
            })
        })
        .flatten();
    // One read for the page rather than one per row: the covers come back
    // keyed by their product, and a row whose product is absent from that set
    // has no stored cover to name.
    let ids: Vec<ProductId> = rows.iter().map(|row| row.id).collect();
    let covered: HashSet<ProductId> = ProductRepo::new(state.pool.clone())
        .covers(context.org, &ids)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .into_iter()
        .map(|cover| cover.product)
        .collect();
    Ok(Json(ProductsPage {
        products: rows
            .into_iter()
            .map(|row| ProductHead {
                cover: covered
                    .contains(&row.id)
                    .then(|| cover_url(&version, row.id)),
                id: row.id,
                title: row.title.0,
                price: row.price,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect(),
        next_cursor,
    }))
}

/// Where one product's cover is fetched from, under the version the caller
/// asked this catalogue for.
///
/// Built from the request's own version rather than from a constant, so a
/// client speaking `/v1` is never handed a `/v2` URL by a server that serves
/// both.
fn cover_url(version: &str, product: ProductId) -> String {
    format!("/{version}/products/{}/cover", product.0.to_hyphenated())
}

/// The eight bytes every PNG begins with.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The type these bytes are, or `None` for bytes this route will not serve.
///
/// Four formats, and each is decided from the bytes rather than from anything
/// a client said: the seller's own thumbnails arrive through `POST /uploads`
/// as whatever their machine produced, and the create form already tells them
/// a JPEG, a PNG or a GIF is acceptable. Serving PNG alone made an edit form
/// unable to draw back three of the four it accepts.
///
/// Deliberately narrow. SVG is absent and stays absent: it is a document that
/// executes script in the browser, so serving one from a seller's own upload
/// under this organisation's origin would be a stored-XSS surface rather than
/// a picture. Anything not listed here is refused rather than served under a
/// type nobody has to believe.
fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&PNG_SIGNATURE) {
        return Some("image/png");
    }
    // SOI plus the first marker's own leading byte. Every JPEG variant this
    // reads — JFIF, Exif, raw — begins the same three bytes.
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    // A RIFF container whose form type is WEBP. Both halves are checked
    // because RIFF also carries WAV and AVI, and a WAV served as an image is
    // the mislabelling this function exists to prevent.
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// The headers an image answer carries, or the refusal for bytes that are not
/// one.
///
/// Both byte-serving routes go through this, so neither can drift into
/// answering a seller's PDF under a type a browser will sniff its way past.
/// `nosniff` is the second half of that: naming the type is worth nothing if
/// the browser is free to disagree with it, and these bytes are a seller's own
/// upload rather than anything this server composed.
pub(crate) fn image_answer(
    bytes: Vec<u8>,
) -> Result<([(header::HeaderName, &'static str); 3], Vec<u8>), APIError> {
    let Some(kind) = image_type(&bytes) else {
        return Err(APIError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            APIErrorEntry::new("these bytes are not an image, and this route serves images only")
                .kind(APIErrorKind::Validation),
        ));
    };
    Ok((
        [
            (header::CONTENT_TYPE, kind),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            // A cover is derived from bytes that are already sealed and never
            // rewritten in place, so it is safe to hold; private because it is
            // one seller's own file and no shared cache may keep it.
            (header::CACHE_CONTROL, "private, max-age=300"),
        ],
        bytes,
    ))
}

/// The bytes of one blob this organisation has sealed, named by its handle.
///
/// The create form's own read: a cover is generated during `POST /uploads`,
/// before any product exists, so there is no product to address it through and
/// the handle the upload already answered is what the form holds. A saved
/// resource's cover is [`product_cover`] instead.
///
/// The organisation is the whole fence, and it is enough. `blob` is keyed
/// `(org_id, hash)` and [`BlobRepo::get`] pins the organisation before it
/// reads, so a handle resolves only inside the tenant that sealed those bytes
/// and another tenant's handle answers exactly as one that does not exist.
///
/// Images only, decided from the bytes by [`image_type`]. A handle names a
/// payload as readily as a thumbnail, and a seller's PDF is not something an
/// image route should stream even back to its owner, so anything that is not
/// one of the four formats is refused by name rather than served under a type
/// it does not have.
pub(crate) async fn uploaded_image(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, handle)): Path<(String, String)>,
) -> Result<([(header::HeaderName, &'static str); 3], Vec<u8>), APIError> {
    let hash = parse_hash(&handle)
        .ok_or_else(|| validation("a handle is the file's 64-character hex hash"))?;
    let Some(blobs) = state.blobs.clone() else {
        return Err(APIError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            APIErrorEntry::new("this deployment holds no object store")
                .kind(APIErrorKind::Internal),
        ));
    };
    let bytes = BlobRepo::new(state.pool.clone(), blobs.object_store(), blobs.kek.clone())
        .get(context.org, hash)
        .await
        .map_err(|error| match error {
            BlobError::Missing => missing("no such handle in this organisation"),
            // Ours, not theirs: the row says these bytes exist and we could not
            // produce them, which is a fault to be seen rather than a resource to
            // be reported absent.
            fault @ (BlobError::Storage(_) | BlobError::Store(_) | BlobError::Crypto(_)) => {
                state.internal(&format!("{fault}"))
            }
        })?;
    image_answer(bytes)
}

/// The bytes of one product's cover.
///
/// The smallest read that lets a browser draw a thumbnail. Nothing else
/// travels with the bytes: the catalogue's own list view already named this
/// URL, and a second description of the file beside it would be a second thing
/// to keep true. Scoped to the caller's organisation by the same context every
/// other catalogue read takes, so one tenant's cover is not reachable from
/// another's session; a product in another organisation answers exactly as a
/// product that does not exist does.
///
/// The type is read off the bytes rather than asserted. Every cover this
/// system stores is a generated PNG — `tam_pipeline::render` encodes one on
/// ingest and the import stores one — so the check passes in every case we
/// write, and anything outside [`image_type`]'s four is refused rather than
/// served under a type nobody has to believe.
pub(crate) async fn product_cover(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, product)): Path<(String, String)>,
) -> Result<([(header::HeaderName, &'static str); 3], Vec<u8>), APIError> {
    let product = ProductId(parse_id(&product)?);
    // The row before the store, so a resource that has no cover answers the
    // same way wherever it is served from: a deployment without an object
    // store is a different fault from a resource with nothing to draw, and
    // asking after the store first would report the first as the second.
    let cover = ProductRepo::new(state.pool.clone())
        .covers(context.org, &[product])
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .into_iter()
        .next()
        .ok_or_else(|| missing("this product has no stored cover"))?;
    let Some(blobs) = state.blobs.clone() else {
        return Err(APIError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            APIErrorEntry::new("this deployment holds no object store")
                .kind(APIErrorKind::Internal),
        ));
    };
    let bytes = BlobRepo::new(state.pool.clone(), blobs.object_store(), blobs.kek.clone())
        .get(context.org, cover.hash)
        .await
        .map_err(|error| match error {
            // The row named a blob the store does not have. That is ours, and a
            // 404 here would say the resource has no cover when its own row says
            // otherwise.
            BlobError::Missing => {
                state.internal("a cover row names a blob this store does not hold")
            }
            fault @ (BlobError::Storage(_) | BlobError::Store(_) | BlobError::Crypto(_)) => {
                state.internal(&format!("{fault}"))
            }
        })?;
    image_answer(bytes)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProductView {
    pub id: ProductId,
    pub title: String,
    pub body: String,
    /// How `body` is written. Carried because the edit path writes it back
    /// and a client that could not read it would have to guess, which is the
    /// sniffing the declaration exists to avoid.
    pub body_format: CopyFormat,
    pub price: PriceIntent,
    /// Payload, cover and preview files alike. Previews are empty for
    /// anything this flow authors — `ingest` produces none — but the field is
    /// named `files` and silently dropping a role would make it a lie.
    pub files: Vec<FileView>,
    pub subjects: Vec<CanonicalTermId>,
    pub grades: GradesView,
    /// The rights grant the source stated, or absent where none was captured.
    /// Carried for the same reason `body_format` is: the edit path writes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rights: Option<PathView>,
    /// The TPT-base sidecar, or `None` for a product that has no row.
    ///
    /// Carried on this read rather than served from a second endpoint: the
    /// edit form needs the product and the sidecar together to render one set
    /// of fields, and a second round trip would buy nothing. Absent is the
    /// honest answer for a product authored before the table existed or
    /// imported from a marketplace, which the repository already distinguishes
    /// from a row of nulls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tpt_base: Option<TptBaseView>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// The sidecar as the edit form reads it back.
///
/// Serialises to exactly the shape [`tam_authoring::TptBaseInput`]
/// deserialises from, so the seed and the request body speak one vocabulary
/// and a round trip through the form is checkable rather than assumed. Every
/// field is the wire's own name; nothing here is presentation.
#[derive(Debug, Serialize, Deserialize)]
pub struct TptBaseView {
    pub thumbnail_mode: u8,
    pub thumbnail_hashes: Vec<String>,
    pub video_preview_hash: Option<String>,
    pub additional_licence_minor_units: Option<i64>,
    pub bundle_discount_minor_units: Option<i64>,
    pub tax_code_id: Option<u8>,
    pub subject_areas: Vec<String>,
    pub tags: Vec<String>,
    pub formats: Vec<String>,
    pub custom_categories: Vec<String>,
    /// Three-state, as the column is: `None` states nothing about the control
    /// rather than answering it unticked.
    pub appropriate_for_country: Option<bool>,
    pub standards: Vec<StandardView>,
    pub teaching_duration_id: Option<u8>,
    pub pages_or_slides: Option<u32>,
    pub answer_key_id: Option<u8>,
    pub copyright_declaration_id: Option<u8>,
    pub status_user: u8,
}

/// One alignment, named as the input names it: the framework by TPT's own
/// jurisdiction id rather than by the domain's enum, because that is the
/// number the create body carries and the one a round trip has to preserve.
#[derive(Debug, Serialize, Deserialize)]
pub struct StandardView {
    pub framework: u32,
    pub code: String,
    pub tpt_node_id: Option<u64>,
}

fn tpt_base_view(record: &tam_storage::TptBaseRecord) -> TptBaseView {
    TptBaseView {
        thumbnail_mode: record.thumbnail_mode.wire_id(),
        thumbnail_hashes: record
            .thumbnails
            .iter()
            .map(|handle| handle.as_str().to_owned())
            .collect(),
        video_preview_hash: record
            .video_preview
            .as_ref()
            .map(|handle| handle.as_str().to_owned()),
        additional_licence_minor_units: record.additional_licence_minor_units,
        bundle_discount_minor_units: record.bundle_discount_minor_units,
        tax_code_id: record.tax_code.map(tam_domain::product::TaxCode::wire_id),
        subject_areas: facet_strings(&record.categories.subject_areas),
        tags: facet_strings(&record.categories.tags),
        formats: facet_strings(&record.categories.formats),
        custom_categories: record.categories.custom_categories.clone(),
        appropriate_for_country: record.categories.appropriate_for_country,
        standards: record
            .standards
            .iter()
            .map(|alignment| StandardView {
                framework: alignment.framework.jurisdiction_id(),
                code: alignment.code.clone(),
                tpt_node_id: alignment.tpt_node_id,
            })
            .collect(),
        teaching_duration_id: record
            .details
            .teaching_duration
            .map(tam_domain::product::TeachingDuration::wire_id),
        pages_or_slides: record.details.pages_or_slides,
        answer_key_id: record
            .details
            .answer_key
            .map(tam_domain::product::AnswerKey::wire_id),
        copyright_declaration_id: record
            .copyright
            .map(tam_domain::product::CopyrightDeclaration::wire_id),
        status_user: record.status.wire_id(),
    }
}

/// The stored sidecar as the input a create or an edit would have sent.
///
/// So that a rule needing a whole draft can read one for a product whose edit
/// carries no block of its own. Built from [`tpt_base_view`] rather than from
/// the record a second time, because a second decoding of the same wire ids is
/// a second thing to drift from the row.
pub(crate) fn tpt_base_input(record: &tam_storage::TptBaseRecord) -> TptBaseInput {
    let view = tpt_base_view(record);
    TptBaseInput {
        thumbnail_mode: Some(view.thumbnail_mode),
        thumbnail_hashes: view.thumbnail_hashes,
        video_preview_hash: view.video_preview_hash,
        additional_licence_minor_units: view.additional_licence_minor_units,
        bundle_discount_minor_units: view.bundle_discount_minor_units,
        tax_code_id: view.tax_code_id,
        subject_areas: view.subject_areas,
        tags: view.tags,
        formats: view.formats,
        custom_categories: view.custom_categories,
        appropriate_for_country: view.appropriate_for_country,
        standards: view
            .standards
            .into_iter()
            .map(|standard| StandardInput {
                framework: standard.framework,
                code: standard.code,
                tpt_node_id: standard.tpt_node_id,
            })
            .collect(),
        teaching_duration_id: view.teaching_duration_id,
        pages_or_slides: view.pages_or_slides,
        answer_key_id: view.answer_key_id,
        copyright_declaration_id: view.copyright_declaration_id,
        status_user: Some(view.status_user),
    }
}

fn facet_strings(slugs: &[tam_domain::product::FacetSlug]) -> Vec<String> {
    slugs.iter().map(|slug| slug.as_str().to_owned()).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileView {
    pub id: Uuid,
    pub role: String,
    pub kind: String,
    pub byte_len: u64,
    /// The digest of these bytes, which is how every other surface names a
    /// file: the create form holds one per upload and the sidecar stores one
    /// per thumbnail.
    ///
    /// Carried so an edit form can seed a draft that is genuinely this
    /// product. Without it the form has an id it cannot make a handle from,
    /// and the model's whole-product check — which needs a payload to build a
    /// product at all — goes silent on the one path where the seller is
    /// changing the fields it governs. Whichever arm holds the bytes vouched
    /// for it, which `scan_vouched_by` beside it already discloses.
    pub hash: String,
    pub scan: String,
    /// Who vouched for `scan`.
    ///
    /// `"server"` where we scanned the bytes ourselves, `"device"` where a
    /// seller's machine did and we never held them. Rendering the two the same
    /// way would put a device's assertion in our own voice, which is the
    /// distinction `hash` and `observed_hash` exist to keep and which Q-b
    /// decided explicitly: the device's scan is acceptable and advisory, and
    /// we do not restate it as a clean bill of ours.
    pub scan_vouched_by: String,
    /// What the seller called this file, where a name was recorded.
    ///
    /// Absent for every file stored before the name column existed, and for a
    /// cover, which is generated rather than chosen. The client renders the
    /// absence rather than substituting the kind, because "PDF" is not a name
    /// and three of them are not three names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Who vouched for a file's scan, from which arm holds its bytes.
fn scan_vouched_by(bytes: &tam_types::FileBytes) -> &'static str {
    match bytes {
        tam_types::FileBytes::Held { .. } => "server",
        tam_types::FileBytes::Sourced { .. } => "device",
    }
}

/// One stored file as every reader of one renders it. Shared with the file
/// routes in `catalogue`, so a field added here reaches the read and the three
/// writes at once rather than three of the four.
pub(crate) fn file_view(file: &tam_types::ProductFile, name: Option<&str>) -> FileView {
    FileView {
        id: file.id.0,
        role: role_str(file.role).to_owned(),
        kind: kind_str(file.kind).to_owned(),
        byte_len: file.bytes.byte_len(),
        hash: crate::catalogue::hash_hex(file.bytes.digest()),
        scan: scan_str(file.bytes.scan()).to_owned(),
        scan_vouched_by: scan_vouched_by(&file.bytes).to_owned(),
        name: name.map(str::to_owned),
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GradesView {
    pub source: String,
    pub raw: Vec<PathView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived: Option<AgeView>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgeView {
    pub low_years: u8,
    pub high_years: u8,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PathView {
    pub inventory: InventoryId,
    pub kind: String,
    pub segments: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_id: Option<String>,
}

const fn term_kind_str(kind: TermKind) -> &'static str {
    match kind {
        TermKind::Subject => "subject",
        TermKind::Topic => "topic",
        TermKind::ResourceType => "resource_type",
        TermKind::Phase => "phase",
        TermKind::Licence => "licence",
    }
}

fn path_view(path: &VocabularyPath) -> PathView {
    let VocabularyId(inventory, kind) = path.vocabulary;
    PathView {
        inventory,
        kind: term_kind_str(kind).to_owned(),
        segments: path.segments.clone(),
        native_id: path.native_id.clone(),
    }
}

const fn scan_str(scan: &ScanOutcome) -> &'static str {
    match scan {
        ScanOutcome::Pending => "pending",
        ScanOutcome::Clean { .. } => "clean",
        ScanOutcome::Infected { .. } => "infected",
        ScanOutcome::Failed { .. } => "failed",
    }
}

pub const fn role_str(role: tam_types::FileRole) -> &'static str {
    match role {
        tam_types::FileRole::Payload => "payload",
        tam_types::FileRole::Preview => "preview",
        tam_types::FileRole::Cover => "cover",
    }
}

pub const fn kind_str(kind: tam_types::FileKind) -> &'static str {
    match kind {
        tam_types::FileKind::Pdf => "pdf",
        tam_types::FileKind::Pptx => "pptx",
        tam_types::FileKind::Docx => "docx",
        tam_types::FileKind::Zip => "zip",
        tam_types::FileKind::Image => "image",
    }
}

/// The inverse of [`kind_str`], so an upload handle names its kind in the
/// same vocabulary the product view reads it back in.
pub(crate) fn kind_from_str(raw: &str) -> Option<tam_types::FileKind> {
    tam_types::FileKind::ALL
        .into_iter()
        .find(|kind| kind_str(*kind) == raw)
}

pub(crate) async fn product_view(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, product)): Path<(String, String)>,
) -> Result<Json<ProductView>, APIError> {
    let product = ProductId(parse_id(&product)?);
    let record = ProductRepo::new(state.pool.clone())
        .get(context.org, product)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such product"))?;
    let tpt_base = tam_storage::TptBaseRepo::new(state.pool.clone())
        .get(context.org, product)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let file_names = record.file_names;
    let aggregate = record.product;
    let named = |file: &tam_types::ProductFile| {
        file_view(file, file_names.get(&file.id).map(String::as_str))
    };
    let mut files: Vec<FileView> = aggregate.payload_files().map(&named).collect();
    files.extend(aggregate.cover.iter().map(&named));
    files.extend(aggregate.previews.iter().map(&named));
    let grades = GradesView {
        source: match aggregate.grades.source {
            tam_domain::DeclarationSource::Seller => "seller".to_owned(),
            tam_domain::DeclarationSource::Imported { vocabulary } => {
                let VocabularyId(inventory, kind) = vocabulary;
                format!("imported:{:?}:{}", inventory, term_kind_str(kind))
            }
        },
        raw: aggregate.grades.raw.iter().map(path_view).collect(),
        derived: aggregate.grades.derived.map(|interval| AgeView {
            low_years: interval.low_years(),
            high_years: interval.high_years(),
        }),
    };
    let rights = match &aggregate.rights {
        tam_domain::RightsDeclaration::Unstated => None,
        tam_domain::RightsDeclaration::Declared { source } => Some(path_view(source)),
    };
    Ok(Json(ProductView {
        id: aggregate.id,
        title: aggregate.title.0,
        body: aggregate.body.body,
        body_format: aggregate.body.format,
        price: aggregate.price,
        files,
        subjects: aggregate.subjects,
        grades,
        rights,
        tpt_base: tpt_base.as_ref().map(tpt_base_view),
        created_at: record.created_at,
        updated_at: record.updated_at,
    }))
}

// -------------------------------------------------------------- connections

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionsView {
    pub connections: Vec<ConnectionView>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionView {
    pub id: ConnectionId,
    pub marketplace: Marketplace,
    /// Which branch of the automation rule this row's marketplace falls in,
    /// so the client renders the badge from the server's decision rather than
    /// from a second copy of the mapping in TypeScript. Derived from
    /// `marketplace`, never stored.
    pub transport: TransportClass,
    /// The stored link state, unchanged. Kept beside `status` rather than
    /// replaced by it: the two answer different questions, and an operator
    /// reading this resource still needs the row's own state.
    pub state: String,
    /// Whether the connection is actually carrying work, which `state` alone
    /// cannot say. This is the field the client renders.
    pub status: ConnectionStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// The seller's own authorship declaration for this marketplace, absent
    /// where none stands.
    ///
    /// Read whatever state the connection is in, unlike the claim's own read:
    /// a declaration made before any device linked, or standing while the
    /// connection needs a fresh sign-in, is still on record and a seller
    /// looking to check it must see it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorship: Option<AuthorshipView>,
    /// The Tes market this connection authors into, absent on every other
    /// marketplace. Served so a second Tes market is a value this field
    /// carries rather than a second inventory; no surface renders it yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

/// What this surface knows about the seller's declaration for one marketplace.
///
/// Three states rather than two, and the third is the absence of this whole
/// field. A seller who has not declared is `undeclared`, which is a fact about
/// them; a surface that does not serve declarations at all omits the field,
/// which is a fact about the surface. Collapsing those into one `null` would
/// have the operator view state "no declaration" about every seller, which is
/// false about all of them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AuthorshipView {
    Undeclared,
    Declared {
        name: String,
        attested_at: Timestamp,
    },
}

pub(crate) async fn list_connections(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<ConnectionsView>, APIError> {
    let now = (state.wall)();
    let rows = ConnectionRepo::new(state.pool.clone())
        .list(context.org, now)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    // One read for every declaration this tenant holds, rather than one per
    // row: the page renders a row per marketplace and a per-row read would
    // make it cost a query per connection.
    // A list rather than a map: the closed marketplace set is three, so a
    // linear find costs less than the ordering a map would need.
    let declared: Vec<(Marketplace, AuthorshipView)> = ConnectionFactsRepo::new(state.pool.clone())
        .declarations(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .into_iter()
        .map(|(marketplace, record)| {
            (
                marketplace,
                AuthorshipView::Declared {
                    name: record.name,
                    attested_at: record.attested_at,
                },
            )
        })
        .collect();
    Ok(Json(ConnectionsView {
        connections: rows
            .into_iter()
            .map(|row| ConnectionView {
                // Always a value on the seller's own surface: this endpoint
                // serves declarations, so a marketplace with none is
                // `undeclared` rather than unknown.
                authorship: Some(
                    declared
                        .iter()
                        .find(|(marketplace, _)| *marketplace == row.marketplace)
                        .map_or(AuthorshipView::Undeclared, |(_, view)| view.clone()),
                ),
                id: row.id,
                marketplace: row.marketplace,
                transport: row.marketplace.transport_class(),
                state: row.state,
                status: row.status,
                created_at: row.created_at,
                updated_at: row.updated_at,
                country: (row.marketplace == Marketplace::Tes).then_some(row.country),
            })
            .collect(),
    }))
}

/// What a revocation did, in the shape the broker's own answer had.
///
/// `connections` counted what the broker tombstoned and now counts what this
/// call moved, which is one or zero; the shape is kept because re-pointing
/// the route should not make the client's reading of it change.
#[derive(Debug, Serialize, Deserialize)]
pub struct RevokedView {
    pub connections: u32,
    pub elapsed_ms: i64,
}

/// Revokes one connection, which is now a write on the row rather than a
/// round trip to a process holding a credential.
///
/// The revocation is terminal without needing anything else to enforce it: the
/// device check-in lifts a connection to `linked` only from `unlinked`,
/// `linking` or `needs_reauth`, so a seller's machines can go on reporting
/// live sessions and none of them restores a revoked one.
pub(crate) async fn revoke_connection(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, connection)): Path<(String, String)>,
) -> Result<Json<RevokedView>, APIError> {
    let connection = ConnectionId(parse_id(&connection)?);
    let now = (state.wall)();
    let revoked = ConnectionRepo::new(state.pool.clone())
        .revoke(
            context.org,
            connection,
            Stamp {
                at: now,
                actor: Actor::Person(context.user),
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(RevokedView {
        connections: u32::from(revoked),
        elapsed_ms: ((state.wall)().0 - now.0).max(0),
    }))
}

/// Disconnects one connection at the seller's own request.
///
/// The seller's verb, and deliberately not `revoke`'s. This writes `unlinked`,
/// which the device check-in lifts back to `linked`, so a seller who
/// disconnects a marketplace and connects it again on their machine takes the
/// same path they took the first time. `revoke` above stays terminal and stays
/// the operator and security path.
///
/// The answer is `RevokedView` unchanged: one call, one row moved or none, and
/// a second shape saying the same two numbers would be a second thing for the
/// client to read.
///
/// A connection the tenant does not have answers `connections: 0` rather than
/// a not-found, because the tenant pin means the statement cannot see another
/// tenant's row and must not distinguish one from a row that is not there.
pub(crate) async fn disconnect_connection(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, connection)): Path<(String, String)>,
) -> Result<Json<RevokedView>, APIError> {
    let connection = ConnectionId(parse_id(&connection)?);
    let now = (state.wall)();
    let unlinked = ConnectionRepo::new(state.pool.clone())
        .unlink(
            context.org,
            connection,
            Stamp {
                at: now,
                actor: Actor::Person(context.user),
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(RevokedView {
        connections: u32::from(unlinked),
        elapsed_ms: ((state.wall)().0 - now.0).max(0),
    }))
}

// ----------------------------------------------------------- reconciliation

#[derive(Debug, Serialize, Deserialize)]
pub struct QueueView {
    pub items: Vec<QueueItemView>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueueItemView {
    pub id: Uuid,
    pub term: CanonicalTermId,
    pub inventory: InventoryId,
    pub kind: String,
    pub raised_at: Timestamp,
}

pub(crate) async fn list_queue(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<QueueView>, APIError> {
    let items = TaxonomyRepo::new(state.pool.clone())
        .open_items(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(QueueView {
        items: items
            .into_iter()
            .map(|item| {
                let VocabularyId(inventory, kind) = item.target;
                QueueItemView {
                    id: item.id,
                    term: item.term,
                    inventory,
                    kind: term_kind_str(kind).to_owned(),
                    raised_at: item.raised_at,
                }
            })
            .collect(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ResolveBody {
    pub segments: Vec<String>,
    #[serde(default)]
    pub native_id: Option<String>,
    /// `exact` or `broader`, defaulting to `exact` so the existing clients do
    /// not move.
    ///
    /// It has to be stated because `projection_edge` is global: the first
    /// seller to answer an uncovered grade would otherwise write an `Exact`
    /// that becomes every tenant's equivalence, and
    /// `projection_edge_exact_reverse` then locks the correct term out of that
    /// path forever. `narrower` is refused: a narrower target invents a
    /// distinction the source does not carry, and the projection would never
    /// read the edge anyway.
    #[serde(default = "default_edge_kind")]
    pub kind: String,
}

fn default_edge_kind() -> String {
    "exact".to_owned()
}

pub(crate) async fn resolve_item(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, item)): Path<(String, String)>,
    Json(body): Json<ResolveBody>,
) -> Result<StatusCode, APIError> {
    check_path(&body.segments)?;
    let kind = match body.kind.as_str() {
        "exact" => EdgeKind::Exact,
        "broader" => EdgeKind::Broader,
        "narrower" => {
            return Err(validation(
                "a narrower edge invents a distinction the source does not carry, so the \
                 projection never reads one; record the broader direction instead",
            ));
        }
        other => return Err(validation(&format!("unknown edge kind {other}"))),
    };
    let item_id = parse_id(&item)?;
    let repo = TaxonomyRepo::new(state.pool.clone());
    let open = repo
        .open_items(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let target = open
        .into_iter()
        .find(|candidate| candidate.id == item_id)
        .ok_or_else(|| missing("no such open item"))?;
    let edge = ProjectionEdge {
        from: target.term,
        to: VocabularyPath {
            vocabulary: target.target,
            segments: body.segments,
            native_id: body.native_id,
        },
        kind,
        decided_by: Decider::Human {
            user: context.user,
            org: context.org,
        },
        decided_at: (state.wall)(),
    };
    // The edge is durable, global and permanent: `projection_edge` carries no
    // organisation, both uniqueness indexes refuse a corrected row, and
    // nothing deletes one. The answer shape makes the native id optional and
    // the shipped client sends none, so without this the ordinary resolution
    // of a TPT tag-axis item writes the exact unaddressed edge every tenant's
    // cross-listing then refuses at the write model, silently and for good.
    // The seeder runs the same check over the relation it derives.
    if let Err(foreign) = check_native_ids(std::slice::from_ref(&edge)) {
        return Err(validation(
            &foreign
                .0
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    repo.resolve_with_edge(context.org, item_id, &edge)
        .await
        .map_err(|error| {
            if let tam_storage::StorageError::Inconsistent { reason } = &error {
                APIError::new(
                    StatusCode::CONFLICT,
                    APIErrorEntry::new(reason).kind(APIErrorKind::Validation),
                )
            } else {
                storage_fault(&state, &error)
            }
        })?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn no_counterpart_item(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, item)): Path<(String, String)>,
) -> Result<StatusCode, APIError> {
    let item_id = parse_id(&item)?;
    TaxonomyRepo::new(state.pool.clone())
        .resolve_no_counterpart(
            context.org,
            item_id,
            &Decider::Human {
                user: context.user,
                org: context.org,
            },
            (state.wall)(),
        )
        .await
        .map_err(|error| {
            if matches!(error, tam_storage::StorageError::Inconsistent { .. }) {
                missing("no such open item")
            } else {
                storage_fault(&state, &error)
            }
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatsView {
    pub open: u64,
    pub resolved: u64,
    pub no_counterpart: u64,
}

pub(crate) async fn queue_stats(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<StatsView>, APIError> {
    let DrainStats {
        open,
        resolved,
        no_counterpart,
    } = TaxonomyRepo::new(state.pool.clone())
        .drain_stats(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(StatsView {
        open,
        resolved,
        no_counterpart,
    }))
}

// ----------------------------------------------------------------- mappings

#[derive(Debug, Serialize, Deserialize)]
pub struct MappingsView {
    pub mappings: Vec<MappingHeadView>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MappingHeadView {
    pub id: MappingId,
    pub product: ProductId,
    pub inventory: InventoryId,
    pub binding_state: String,
    pub lifecycle_state: String,
    pub updated_at: Timestamp,
    /// The listing's own page on the marketplace, where the binding names one
    /// and the marketplace's page shape is known. Derived from the identifier
    /// the binding already holds; no request is made to produce it.
    pub listing_url: Option<String>,
}

impl MappingHeadView {
    #[must_use]
    pub fn of(row: tam_storage::MappingHead) -> Self {
        Self {
            id: row.id,
            product: row.product,
            inventory: row.inventory,
            binding_state: row.binding_state,
            lifecycle_state: row.lifecycle_state,
            updated_at: row.updated_at,
            listing_url: row.remote.as_ref().and_then(listing_url),
        }
    }
}

pub(crate) async fn list_mappings(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<MappingsView>, APIError> {
    let rows = tam_storage::MappingRepo::new(state.pool.clone())
        .list_heads(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(MappingsView {
        mappings: rows.into_iter().map(MappingHeadView::of).collect(),
    }))
}

// ----------------------------------------------------------- overrides

/// One seller's own mapping decision, as the Templates screen sends it.
///
/// The organisation and the user are deliberately absent: they come from the
/// session through [`OrgContext`], so a body cannot name an organisation it
/// does not speak for.
#[derive(Debug, Clone, Deserialize)]
pub struct OverrideBody {
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub from_term: CanonicalTermId,
    pub to: PathInput,
    /// `exact` or `broader`.
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PathInput {
    pub segments: Vec<String>,
    #[serde(default)]
    pub native_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverrideView {
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub from_term: CanonicalTermId,
    pub segments: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_id: Option<String>,
    pub kind: String,
    pub decided_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverridesView {
    pub overrides: Vec<OverrideView>,
}

/// Which override to withdraw. The triple is the key: one organisation holds
/// at most one override per marketplace, axis and term.
#[derive(Debug, Clone, Deserialize)]
pub struct WithdrawOverrideBody {
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub from_term: CanonicalTermId,
}

fn override_kind_of(raw: &str) -> Option<OverrideKind> {
    match raw {
        "exact" => Some(OverrideKind::Exact),
        "broader" => Some(OverrideKind::Broader),
        _ => None,
    }
}

const fn override_kind_str(kind: OverrideKind) -> &'static str {
    match kind {
        OverrideKind::Exact => "exact",
        OverrideKind::Broader => "broader",
    }
}

/// Records one seller's own answer for how a term of theirs projects.
///
/// The organisation and the user come from the session rather than the body,
/// so `decided_by` names who actually asked. `ProjectionOverride::new` is the
/// only way the value is built, because the licence refusal is half domain and
/// half database CHECK and skipping the constructor would leave the database
/// to answer what the seller should have been told. The native identifier is
/// checked before the write by the same function the reconciliation queue's
/// resolution runs, which is what keeps an identifier of the wrong shape from
/// reaching a live listing.
pub(crate) async fn upsert_override(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<OverrideBody>,
) -> Result<StatusCode, APIError> {
    let kind = override_kind_of(&body.kind)
        .ok_or_else(|| validation("an override is exact or broader"))?;
    // Before the constructor, which checks only that the path is non-empty:
    // the domain's own bound is about meaning, and this one is about what a
    // stranger may make this server write.
    check_path(&body.to.segments)?;
    let to = VocabularyPath {
        vocabulary: VocabularyId(body.inventory, body.axis),
        segments: body.to.segments,
        native_id: body.to.native_id,
    };
    let decided_at = (state.wall)();
    let decided_by = Decider::Human {
        user: context.user,
        org: context.org,
    };

    let entry = ProjectionOverride::new(NewProjectionOverride {
        org: context.org,
        inventory: body.inventory,
        axis: body.axis,
        from: body.from_term,
        to: to.clone(),
        kind,
        decided_by: decided_by.clone(),
        decided_at,
    })
    .map_err(|error| match error {
        ProjectionOverrideError::LicenceNeverOverridden => validation(
            "a licence is a legal statement about the work rather than a mapping choice, so it is never overridden",
        ),
        ProjectionOverrideError::EmptyPath => {
            validation("an override names at least one path segment")
        }
        ProjectionOverrideError::UnboundAxis => validation(
            "this marketplace carries no field for that axis, so an override for it would reach nothing",
        ),
    })?;

    // The same check the reconciliation queue's resolution runs, over the one
    // edge this override would produce. An identifier of the wrong shape is
    // refused where it is authored rather than where it is spent, which is
    // after the seller has already asked for the cross-listing.
    check_native_ids(&[ProjectionEdge {
        from: body.from_term,
        to,
        kind: match kind {
            OverrideKind::Exact => EdgeKind::Exact,
            OverrideKind::Broader => EdgeKind::Broader,
        },
        decided_by,
        decided_at,
    }])
    .map_err(|foreign| {
        APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new(
                "that value's identifier is not one this marketplace issues, so a listing carrying it would be refused",
            )
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({
                "native_ids": foreign
                    .0
                    .iter()
                    .map(|entry| entry.native_id.clone())
                    .collect::<Vec<_>>(),
            })),
        )
    })?;

    OverrideRepo::new(state.pool.clone())
        .upsert(&entry)
        .await
        .map_err(|error| unknown_term_or_fault(&state, &error))?;
    Ok(StatusCode::NO_CONTENT)
}

/// The foreign key `projection_override.from_term` carries onto the canonical
/// taxonomy, under the name Postgres gives an inline `REFERENCES`.
const FROM_TERM_FKEY: &str = "projection_override_from_term_fkey";

/// A term this taxonomy does not hold is the caller's to correct, not a fault
/// of ours.
///
/// It is reachable through an ordinary client: the Templates screen picks from
/// a cached list of terms, so a seller whose tab has been open across a
/// taxonomy change can name one that has since gone. Without this it surfaces
/// as a five-hundred, which tells them nothing and reads as our failure.
fn unknown_term_or_fault(state: &AppState, error: &StorageError) -> APIError {
    if let StorageError::Db(sqlx::Error::Database(database)) = error {
        if database.constraint() == Some(FROM_TERM_FKEY) {
            return validation(
                "that term is not in the taxonomy any more; reload and pick it again",
            );
        }
    }
    storage_fault(state, error)
}

/// The calling organisation's own overrides. No other organisation's are
/// reachable: the repository pins the tenant itself.
pub(crate) async fn list_overrides(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<OverridesView>, APIError> {
    let held = OverrideRepo::new(state.pool.clone())
        .for_org(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(OverridesView {
        overrides: held
            .into_iter()
            .map(|entry| OverrideView {
                inventory: entry.inventory,
                axis: entry.axis,
                from_term: entry.from,
                segments: entry.to.segments,
                native_id: entry.to.native_id,
                kind: override_kind_str(entry.kind).to_owned(),
                decided_at: entry.decided_at,
            })
            .collect(),
    }))
}

/// Withdraws one override, leaving the global relation to answer again.
///
/// A withdrawal that matches nothing is not an error: the seller's intent is
/// that no override stand for that term, and it already does not.
pub(crate) async fn withdraw_override(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<WithdrawOverrideBody>,
) -> Result<StatusCode, APIError> {
    OverrideRepo::new(state.pool.clone())
        .remove(context.org, body.inventory, body.axis, body.from_term)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------- labels

/// One label as the console renders it. The colour is the server's, derived
/// from the name rather than chosen, so one label looks the same everywhere it
/// appears without a seller having to manage a palette.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelView {
    pub name: String,
    pub colour: String,
    /// Whether this label is the marketplace's own rather than the seller's.
    ///
    /// The console renders a system chip with no remove control and leaves it
    /// out of the set it sends back, and the two label counts below leave it
    /// out of the allowance. Carried rather than inferred from the name,
    /// because a seller may legitimately have typed "TPT" themselves and the
    /// difference is a column, not a spelling.
    pub system: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelsView {
    pub labels: Vec<LabelView>,
}

/// The whole set an item carries after this write.
///
/// Whole rather than a delta: the console renders the set and sends it back,
/// and a delta would leave removing the last label with no spelling.
#[derive(Debug, Clone, Deserialize)]
pub struct SetLabelsBody {
    pub labels: Vec<String>,
}

/// The bound on how many labels one item may carry.
///
/// A filing dimension rather than a tagging free-for-all: a seller who needs
/// forty labels on one item is describing something the label is the wrong
/// tool for, and an unbounded list is a row this server writes on a stranger's
/// say-so.
pub(crate) const LABELS_PER_PRODUCT_MAX: usize = 20;

/// The longest a label may be, shared by every route that accepts one so the
/// rename cannot refuse a name the create path mints. Migration 0046 states
/// the same bound on the column.
const LABEL_MAX_CHARS: usize = 60;

/// One label's text as it will be stored, or the refusal a seller can act on.
///
/// The character rules are the address path's, not taste: a label is addressed
/// by its own text as one path segment, so a name carrying `/` would be
/// reachable only as `%2F` and any intermediary that normalises the escape
/// back turns the request into a path matching no route — a label a seller
/// could create and then neither rename nor delete. Control characters go for
/// the reason [`crate::text::is_typed_text`] states. Everything else a person
/// might type is accepted, and a client sends it percent-encoded.
fn validated_label(raw: &str) -> Result<String, APIError> {
    label_refusal(raw).map_err(|refusal| validation(&refusal))
}

/// The label rules themselves, answering the refusal as plain words.
///
/// Split out from [`validated_label`] so that the spreadsheet import can apply
/// exactly these rules without going through an `APIError`: its report cites a
/// sheet, a row and a column, and needs the sentence rather than a response.
/// One function with two callers rather than two copies that agree today —
/// `LABEL_MAX_CHARS` moving here now moves both, which is the property a
/// second copy cannot have however carefully it is written.
pub(crate) fn label_refusal(raw: &str) -> Result<String, String> {
    let name = tam_storage::labels::normalise(raw);
    if name.is_empty() {
        return Err("a label needs a word in it".to_owned());
    }
    if name.chars().count() > LABEL_MAX_CHARS {
        return Err(format!("a label is at most {LABEL_MAX_CHARS} characters"));
    }
    if !crate::text::is_typed_text(&name) {
        return Err("a label cannot contain control characters".to_owned());
    }
    if name.contains('/') {
        return Err(
            "a label cannot contain a slash, because a label is addressed by its own name"
                .to_owned(),
        );
    }
    Ok(name)
}

fn labels_view(records: Vec<tam_storage::LabelRecord>) -> Json<LabelsView> {
    Json(LabelsView {
        labels: records
            .into_iter()
            .map(|record| LabelView {
                name: record.name,
                colour: record.colour.as_str().to_owned(),
                system: record.system,
            })
            .collect(),
    })
}

pub(crate) async fn product_labels(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, product)): Path<(String, String)>,
) -> Result<Json<LabelsView>, APIError> {
    let product = ProductId(parse_id(&product)?);
    product_or_missing(&state, context.org, product).await?;
    let records = LabelRepo::new(state.pool.clone())
        .for_product(context.org, product)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(labels_view(records))
}

pub(crate) async fn set_product_labels(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, product)): Path<(String, String)>,
    Json(body): Json<SetLabelsBody>,
) -> Result<Json<LabelsView>, APIError> {
    let product = ProductId(parse_id(&product)?);
    product_or_missing(&state, context.org, product).await?;

    // The cap is checked against what arrived, before anything walks the list:
    // the deduplication below is quadratic, so checking the bound afterwards
    // would let an unbounded array do unbounded work to earn its refusal.
    if body.labels.len() > LABELS_PER_PRODUCT_MAX {
        return Err(validation(
            "an item carries at most twenty labels; labels file a catalogue rather than describe one item",
        ));
    }

    // Trimmed, emptied and deduplicated here rather than in the repository,
    // because what a seller may type is an API question: the storage layer
    // stores what it is given.
    let mut names: Vec<String> = Vec::with_capacity(body.labels.len());
    for raw in &body.labels {
        let name = validated_label(raw)?;
        if !names
            .iter()
            .any(|held: &String| held.eq_ignore_ascii_case(&name))
        {
            names.push(name);
        }
    }

    // The plan's label allowance is the organisation's whole shelf, not this
    // item's: a label is a filing system, and twenty of them across a
    // catalogue is a different quantity from twenty on one resource. Only
    // names this organisation does not already hold count against it, so
    // re-labelling an item with labels it already has is never refused.
    //
    // The marketplace's own labels are not on that shelf. The seller did not
    // type them and cannot remove them, so spending their allowance on them
    // would charge them for having imported, and a seller at the ceiling would
    // find their next manual label refused by a chip they never asked for.
    let labels = LabelRepo::new(state.pool.clone());
    let every = labels
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    // A system label's name is ours, so naming one here is refused rather than
    // quietly dropped: the console never sends it, and a client that does is
    // asking for a label whose colour and flag this route would not write.
    if let Some(claimed) = names.iter().find(|name| {
        every
            .iter()
            .any(|record| record.system && record.name.eq_ignore_ascii_case(name))
    }) {
        return Err(validation(&format!(
            "{claimed} is the label of a marketplace you imported from; it is set for you and              cannot be typed or removed"
        )));
    }
    let held: Vec<&tam_storage::LabelRecord> =
        every.iter().filter(|record| !record.system).collect();
    let fresh = names
        .iter()
        .filter(|name| {
            !held
                .iter()
                .any(|record| record.name.eq_ignore_ascii_case(name))
        })
        .count();
    let after = held.len().saturating_add(fresh);
    let allowed = usize::try_from(context.entitlement.caps.labels_max).unwrap_or(usize::MAX);
    if fresh > 0 && after > allowed {
        return Err(quota_refusal(
            QuotaKind::Labels,
            i64::try_from(held.len()).unwrap_or(i64::MAX),
            u64::from(context.entitlement.caps.labels_max),
        ));
    }
    let records = labels
        .set_for_product(context.org, product, &names, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(labels_view(records))
}

/// Every label this organisation uses, which is what the board's filter lists.
pub(crate) async fn list_labels(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<LabelsView>, APIError> {
    let records = LabelRepo::new(state.pool.clone())
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(labels_view(records))
}

/// The new name for a label, which is the whole of a rename: a label has no
/// other field a seller may set, because the colour is derived from the name.
#[derive(Debug, Clone, Deserialize)]
pub struct RenameLabelBody {
    pub name: String,
}

/// Renames one label, keeping every item that carries it.
///
/// The label is addressed by its own text, matched the way the unique index
/// matches it — case-insensitively — because that is the only identifier this
/// surface ever gives a client for one. A name carrying anything a URL path
/// reserves is percent-encoded by the client; `/` cannot appear in a name at
/// all, which [`validated_label`] refuses at the point one is minted.
///
/// The answer is the label as stored: the name is trimmed on the way in and
/// the colour is recomputed from it, so a client that rendered what it sent
/// would show a colour this organisation's label does not have.
pub(crate) async fn rename_label(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, name)): Path<(String, String)>,
    Json(body): Json<RenameLabelBody>,
) -> Result<Json<LabelView>, APIError> {
    let to = validated_label(&body.name)?;
    let from = tam_storage::labels::normalise(&name);
    let outcome = LabelRepo::new(state.pool.clone())
        .rename(context.org, &from, &to)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match outcome {
        LabelRename::Renamed(record) => Ok(Json(LabelView {
            name: record.name,
            colour: record.colour.as_str().to_owned(),
            system: record.system,
        })),
        LabelRename::Missing => Err(missing("no label of that name")),
        // A refusal rather than a merge. Folding the two labels together
        // would take every item off one of them, which is a bulk edit of the
        // catalogue rather than the rename that was asked for, and no call
        // here says whether that is what the seller meant.
        LabelRename::Taken => Err(validation("that name is already one of your labels")),
    }
}

/// Removes one label from this organisation, and with it from every item
/// carrying it.
///
/// 204 rather than the emptied set: nothing of the label survives the call, so
/// there is no representation to answer with.
pub(crate) async fn delete_label(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, name)): Path<(String, String)>,
) -> Result<StatusCode, APIError> {
    let name = tam_storage::labels::normalise(&name);
    let removed = LabelRepo::new(state.pool.clone())
        .delete(context.org, &name)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(missing("no label of that name"))
    }
}

/// Refuses with the same not-found another organisation's product gets, so a
/// label write is never an oracle for which product identifiers exist.
async fn product_or_missing(
    state: &AppState,
    org: OrgId,
    product: ProductId,
) -> Result<(), APIError> {
    ProductRepo::new(state.pool.clone())
        .get(org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("no such product"))?;
    Ok(())
}

// ---------------------------------------------------------------- bind

/// The listing page the seller pasted. One field, because everything else a
/// bind needs is either the mapping's own or the server's to decide: which
/// marketplace it must name comes from the mapping, and the verification state
/// it starts in is not a client's to choose.
#[derive(Debug, Clone, Deserialize)]
pub struct BindMappingBody {
    pub listing_url: String,
}

/// Binds a mapping to a listing this tree did not create.
///
/// The seller's own catalogue is the only thing written: the URL is parsed to
/// the marketplace's identifier and stored, and no marketplace is contacted
/// here or anywhere on this path. The binding therefore starts unverified —
/// `Verification::Stale` at the bind instant, which is the state migration
/// 0004 gives a bound mapping nothing has read back — and the engine's own
/// read-back is what confirms the listing exists and matches. A seller can
/// paste a listing that is gone or is not theirs; that read-back is what
/// catches it, and until then the mapping states a claim rather than a fact.
pub(crate) async fn bind_mapping(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, mapping)): Path<(String, String)>,
    Json(body): Json<BindMappingBody>,
) -> Result<Json<MappingHeadView>, APIError> {
    let mapping = MappingId(parse_id(&mapping)?);
    let repo = MappingRepo::new(state.pool.clone());
    let head = repo
        .head(context.org, mapping)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such mapping"))?;

    let listing = parse_listing_url(head.inventory.marketplace(), body.listing_url.trim())
        .map_err(|refusal| unusable_url(head.inventory, refusal))?;

    let outcome = repo
        .bind_pasted(context.org, mapping, &listing, (state.wall)())
        .await
        .map_err(|error| match error {
            // The guarded UPDATE reports a vanished mapping this way, which is
            // a race with a delete rather than a fault of ours.
            StorageError::Inconsistent { .. } => missing("no such mapping"),
            other @ (StorageError::Db(_)
            | StorageError::TimestampOutOfRange { .. }
            | StorageError::CorruptRow { .. }
            | StorageError::OrgMismatch
            | StorageError::SellerRuleBlocked { .. }
            | StorageError::StaleLease
            | StorageError::DuplicateIdempotencyKey { .. }
            | StorageError::AttemptInFlight
            | StorageError::MappingAlreadyBound
            | StorageError::InventoryMappingAlreadyExists
            | StorageError::StorefrontBoundElsewhere { .. }
            | StorageError::ListingAlreadyBound) => storage_fault(&state, &other),
        })?;
    match outcome {
        PastedBind::Bound => {}
        PastedBind::NotUnbound { state: binding } => {
            return Err(APIError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                APIErrorEntry::new(match binding.as_str() {
                    "bound" => "this item is already listed on that marketplace",
                    "severed" => {
                        "this item's listing there was cut loose, which reconciliation reattaches rather than a paste"
                    }
                    _ => "a send is already out for this item on that marketplace",
                })
                .code(APIErrorCode::MappingNotBindable)
                .kind(APIErrorKind::Validation)
                .detail(serde_json::json!({ "state": binding })),
            ));
        }
        PastedBind::ListingClaimed => {
            return Err(APIError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                APIErrorEntry::new("another of your items already claims that listing")
                    .code(APIErrorCode::ListingAlreadyClaimed)
                    .kind(APIErrorKind::Validation),
            ));
        }
    }

    let bound = repo
        .head(context.org, mapping)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| state.internal("the mapping just bound was not readable"))?;
    Ok(Json(MappingHeadView::of(bound)))
}

fn unusable_url(inventory: InventoryId, refusal: UrlRefusal) -> APIError {
    let (message, named) = match refusal {
        UrlRefusal::WrongMarketplace { named } => (
            "that link is a listing on a different marketplace",
            Some(named),
        ),
        UrlRefusal::Unrecognised => ("that link is not a listing page we can read", None),
    };
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message)
            .code(APIErrorCode::ListingUrlUnusable)
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({
                "expected": inventory.marketplace(),
                "named": named,
            })),
    )
}

// ------------------------------------------------------------------- status

/// The public per-marketplace status: every inventory the closed set knows,
/// with its halt if one is raised. Deliberately session-free — a status page
/// exists precisely for when logging in is what is broken — and it carries
/// no tenant data, only the fleet kill switch's own state.
#[derive(Debug, Serialize, Deserialize)]
pub struct StatusView {
    pub inventories: Vec<InventoryStatusView>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InventoryStatusView {
    pub inventory: InventoryId,
    pub marketplace: Marketplace,
    /// Which branch of the automation rule this inventory's marketplace falls
    /// in, spelled as [`ConnectionView::transport`] spells it. Served here as
    /// well as there because the console's marketplace page renders a row per
    /// inventory rather than per connection, and a row with no connection
    /// behind it still carries the badge D1 requires.
    pub transport: TransportClass,
    pub halted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raised_at: Option<Timestamp>,
}

const ALL_INVENTORIES: [InventoryId; 3] = [InventoryId::Tes, InventoryId::Etsy, InventoryId::Tpt];

pub(crate) async fn status(State(state): State<AppState>) -> Result<Json<StatusView>, APIError> {
    let halts = tam_storage::HaltRepo::new(state.pool.clone())
        .inventory_halts()
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(StatusView {
        inventories: ALL_INVENTORIES
            .into_iter()
            .map(|inventory| {
                let halt = halts.iter().find(|halt| halt.inventory == inventory);
                InventoryStatusView {
                    inventory,
                    marketplace: inventory.marketplace(),
                    transport: inventory.marketplace().transport_class(),
                    halted: halt.is_some(),
                    reason: halt.map(|halt| halt.reason.clone()),
                    raised_at: halt.map(|halt| halt.raised_at),
                }
            })
            .collect(),
    }))
}

// --------------------------------------------------------------- elections

/// What the decision surface renders, assembled per read rather than fetched.
///
/// The durable question carries no candidate list and no suggestion: a stored
/// list is a snapshot that goes stale the moment a vocabulary is re-polled,
/// and computing the suggestion at exactly one place — here — is what makes
/// "we compute a best fit only where the seller asked us to" a property of one
/// function instead of a convention scattered across writers.
#[derive(Debug, Serialize, Deserialize)]
pub struct DecisionView {
    pub id: Uuid,
    pub product: ProductId,
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub trigger: String,
    pub trigger_key: Option<String>,
    pub raised_at: Timestamp,
    /// `seller_decides` or `best_fit`. A legal axis is always the former,
    /// whatever the tenant has opted into elsewhere.
    pub resolution: String,
    /// The target vocabulary's own members, read out of the relation now.
    pub candidates: Vec<PathView>,
    /// What best fit would pick, where the seller opted into it and there is
    /// a resolved set to rank. Never an answer: an election resolves only on
    /// explicit confirmation.
    pub suggested: Option<SuggestionView>,
    /// What this listing loses on this target whatever the seller picks, so a
    /// Tes-to-TPT licence drop is visible at the moment of decision rather
    /// than after publish.
    pub losses: Vec<LossView>,
}

/// A ranked suggestion: what best fit would keep, and what keeping it leaves
/// behind.
#[derive(Debug, Serialize, Deserialize)]
pub struct SuggestionView {
    /// Highest ranked first, and never more than the target will take.
    pub keep: Vec<RankedView>,
    /// The candidates the target's cardinality left behind, named rather than
    /// dropped quietly. Empty where nothing was dropped.
    pub dropped: Vec<RankedView>,
}

/// One value of the resolved set with the edge the relation reached it by.
#[derive(Debug, Serialize, Deserialize)]
pub struct RankedView {
    pub path: PathView,
    /// `exact`, `broader` or `narrower`. Absent where the relation named no
    /// edge for this value: it is in the resolved set, which is what admits
    /// it to the ranking, and claiming an edge nobody recorded would be a
    /// fact this server made up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge: Option<String>,
}

const fn edge_kind_str(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Exact => "exact",
        EdgeKind::Broader => "broader",
        EdgeKind::Narrower => "narrower",
    }
}

fn ranked_view(ranked: &Ranked) -> RankedView {
    RankedView {
        path: path_view(&ranked.path),
        edge: ranked.edge.map(|kind| edge_kind_str(kind).to_owned()),
    }
}

fn suggestion_view(suggestion: &Suggestion) -> SuggestionView {
    SuggestionView {
        keep: suggestion.keep.iter().map(ranked_view).collect(),
        dropped: suggestion.dropped.iter().map(ranked_view).collect(),
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LossView {
    pub kind: String,
    pub axis: Option<TermKind>,
    pub detail: serde_json::Value,
    pub recorded_at: Timestamp,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DecisionsView {
    pub items: Vec<DecisionView>,
}

/// Which questions the caller wants back.
///
/// Both filters are optional and both narrow: the console asks for the whole
/// decision surface, and one marketplace tab of the authoring form asks
/// "which decisions does this resource still owe this marketplace, and what
/// would best fit pick", which is this route under both filters at once.
#[derive(Debug, Default, Deserialize)]
pub struct DecisionsQuery {
    #[serde(default)]
    pub product: Option<String>,
    /// The same token every JSON body carries, so a client holds one
    /// vocabulary rather than two.
    #[serde(default)]
    pub inventory: Option<String>,
}

/// The relation slices one product's suggestions are ranked over, loaded once
/// per product rather than once per open question.
struct ResolvedContext {
    product: CanonicalProduct,
    edges: Vec<ProjectionEdge>,
    no_counterparts: Vec<(CanonicalTermId, VocabularyId)>,
}

impl ResolvedContext {
    /// What this product's own terms resolve to in one target vocabulary,
    /// which is the set a suggestion ranks and the reason it is not a ranking
    /// of the target's whole vocabulary.
    ///
    /// The seller's own projection overrides are deliberately not consulted.
    /// An override only ever adds to a resolved set, so a set computed
    /// without them is a subset of what the projection resolves — short of
    /// the whole answer, never outside it — and best fit's own invariant, that
    /// it never names a value the resolved set did not hold, survives.
    fn resolved(
        &self,
        kinds: &HashMap<CanonicalTermId, TermKind>,
        vocabulary: VocabularyId,
    ) -> Vec<VocabularyPath> {
        let VocabularyId(_, axis) = vocabulary;
        let terms: Vec<CanonicalTermId> = match axis {
            TermKind::Phase => ingest_grades(&self.product.grades, &self.edges).terms,
            TermKind::Subject | TermKind::Topic | TermKind::ResourceType => self
                .product
                .subjects
                .iter()
                .copied()
                .filter(|term| kinds.get(term) == Some(&axis))
                .collect(),
            // A licence question is always a supply — TPT carries no licence
            // anywhere on its wire, so nothing was stated — and a supply has
            // no resolved set by construction.
            TermKind::Licence => Vec::new(),
        };
        project_terms(&terms, vocabulary, &self.edges, &self.no_counterparts).included
    }
}

/// The question the durable row records, rebuilt with the candidate set it
/// was asked about.
///
/// Two of the four rebuild and two do not, and the two that do not are
/// refusals rather than gaps. A `supply` asks for a value the source never
/// carried, so it has no resolved set by construction and best fit declines
/// on it anyway. A `narrow` asks which values under one source band this
/// listing means, and the row keys on the band's own native id rather than on
/// the term it came from, so the band's candidates cannot be named from the
/// row alone: the question stands rather than being ranked against a set that
/// is not the one it was asked.
fn rebuilt_trigger(
    kind: ElectionTriggerKind,
    binding: AxisBinding,
    resolved: Vec<VocabularyPath>,
) -> Option<ElectionTrigger> {
    match kind {
        ElectionTriggerKind::ElectOne => Some(ElectionTrigger::ElectOne { from: resolved }),
        ElectionTriggerKind::OverCap => match binding.cardinality {
            Cardinality::Many {
                cap: Some(CountCap { limit }),
            } => Some(ElectionTrigger::OverCap {
                cap: limit,
                from: resolved,
            }),
            // The cap that raised the question is the registry's own, so an
            // axis declaring none today cannot be asked what it would keep.
            Cardinality::One | Cardinality::Many { cap: None } => None,
        },
        ElectionTriggerKind::Supply | ElectionTriggerKind::Narrow => None,
    }
}

/// Every axis this tenant has handed to best fit, read off their own standing
/// rules rather than assumed.
///
/// Keyed on `(inventory, axis)` and not on the trigger, because the tick is
/// one permission over a marketplace's axis rather than four separate ones:
/// `election_rule_trigger_key` admits a keyless rule only for the two
/// triggers that generalise to nothing, so a delegation stored per trigger
/// could not cover the two that key. `resolution_for` still has the last
/// word, and `Never` still wins.
fn delegated_axes(rules: &[ElectionRule]) -> HashSet<(InventoryId, TermKind)> {
    rules
        .iter()
        .filter(|rule| rule.answer == ElectionAnswer::Delegate)
        .map(|rule| (rule.inventory, rule.axis))
        .collect()
}

/// One open question rendered, with the suggestion computed here and nowhere
/// else.
fn decision_view(item: OpenElection, context: DecisionContext<'_>) -> DecisionView {
    let binding = registry(item.inventory).axis(item.axis);
    let opted_in = context.delegated.contains(&(item.inventory, item.axis));
    let mode = binding.map_or(Mode::SellerDecides, |binding| {
        resolution_for(binding, opted_in)
    });
    let suggested = binding
        .zip(context.resolved)
        .and_then(|(binding, resolved)| {
            rebuilt_trigger(item.trigger_kind, binding, resolved.to_vec())
                .and_then(|trigger| best_fit(binding, opted_in, &trigger, context.candidates))
        })
        .as_ref()
        .map(suggestion_view);
    DecisionView {
        id: item.id,
        product: item.product,
        inventory: item.inventory,
        axis: item.axis,
        trigger: item.trigger_kind.as_str().to_owned(),
        trigger_key: item.trigger_key,
        raised_at: item.raised_at,
        resolution: match mode {
            Mode::SellerDecides => "seller_decides".to_owned(),
            Mode::BestFit => "best_fit".to_owned(),
        },
        suggested,
        candidates: context
            .candidates
            .iter()
            .filter(|candidate| candidate.edge == EdgeKind::Exact)
            .map(|candidate| path_view(&candidate.path))
            .collect(),
        losses: context.losses,
    }
}

/// What rendering one question needs beyond the row itself, bundled because
/// the arity would otherwise exceed the workspace argument limit.
struct DecisionContext<'a> {
    delegated: &'a HashSet<(InventoryId, TermKind)>,
    /// The target vocabulary's own members with the edge each was reached by,
    /// which is what ranks a suggestion and what the client picks from.
    candidates: &'a [Candidate],
    /// This product's own resolved set in that vocabulary, or `None` where
    /// the product could not be read and so nothing about it can be ranked.
    resolved: Option<&'a [VocabularyPath]>,
    losses: Vec<LossView>,
}

pub(crate) async fn list_decisions(
    State(state): State<AppState>,
    context: OrgContext,
    Query(filter): Query<DecisionsQuery>,
) -> Result<Json<DecisionsView>, APIError> {
    let wanted_product = filter.product.as_deref().map(parse_id).transpose()?;
    let wanted_inventory = filter
        .inventory
        .as_deref()
        .map(|raw| {
            crate::vocabulary::parse_inventory(raw).ok_or_else(|| validation("no such inventory"))
        })
        .transpose()?;
    let repo = ElectionRepo::new(state.pool.clone());
    let taxonomy = TaxonomyRepo::new(state.pool.clone());
    let mappings = MappingRepo::new(state.pool.clone());
    let open: Vec<OpenElection> = repo
        .open_items(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .into_iter()
        .filter(|item| wanted_product.is_none_or(|id| item.product.0 == id))
        .filter(|item| wanted_inventory.is_none_or(|inventory| item.inventory == inventory))
        .collect();
    let delegated = delegated_axes(
        &repo
            .rules(context.org)
            .await
            .map_err(|error| storage_fault(&state, &error))?,
    );
    // Everything below this point exists only to compute a suggestion, and a
    // tenant who has delegated nothing can have none: `best_fit` consults
    // `resolution_for` first and declines. So a seller who has never ticked
    // the control -- which is every seller until they do -- pays one extra
    // read and no more: the `rules` read above happens for every tenant,
    // because whether they have delegated anything is precisely what it
    // answers, and the per-product reads below arrive only with the opt-in
    // that makes them mean something.
    let mut kinds: HashMap<CanonicalTermId, TermKind> = HashMap::new();
    let mut contexts: HashMap<(ProductId, InventoryId), Option<ResolvedContext>> = HashMap::new();
    if !delegated.is_empty() {
        kinds = taxonomy
            .terms()
            .await
            .map_err(|error| storage_fault(&state, &error))?
            .into_iter()
            .map(|term: CanonicalTerm| (term.id, term.kind))
            .collect();
        // Keyed on the pair and loaded before the render loop, so a decision
        // surface holding twenty questions about one listing reads that
        // listing once rather than twenty times.
        for item in &open {
            let key = (item.product, item.inventory);
            if let Entry::Vacant(slot) = contexts.entry(key) {
                slot.insert(resolved_context(&state, context.org, key).await?);
            }
        }
    }
    let mut items = Vec::with_capacity(open.len());
    for item in open {
        let key = (item.product, item.inventory);
        let vocabulary = VocabularyId(item.inventory, item.axis);
        let loaded = contexts.get(&key).and_then(Option::as_ref);
        let candidates = match loaded {
            Some(loaded) => candidates_in(&loaded.edges, vocabulary),
            None => candidates_in(
                &taxonomy
                    .edges_into(vocabulary)
                    .await
                    .map_err(|error| storage_fault(&state, &error))?,
                vocabulary,
            ),
        };
        let resolved = loaded.map(|loaded| loaded.resolved(&kinds, vocabulary));
        let losses = losses_of(&state, &mappings, context.org, item.raised_by).await?;
        items.push(decision_view(
            item,
            DecisionContext {
                delegated: &delegated,
                candidates: &candidates,
                resolved: resolved.as_deref(),
                losses,
            },
        ));
    }
    Ok(Json(DecisionsView { items }))
}

/// The edges into one target vocabulary as ranking candidates. `Narrower` is
/// kept rather than filtered out: a narrow question's candidates are reached
/// by nothing else, and an edge kind carried into the ranking is what lets an
/// exact one outrank a broader one.
fn candidates_in(edges: &[ProjectionEdge], vocabulary: VocabularyId) -> Vec<Candidate> {
    edges
        .iter()
        .filter(|edge| edge.to.vocabulary == vocabulary)
        .map(|edge| Candidate {
            path: edge.to.clone(),
            edge: edge.kind,
        })
        .collect()
}

/// One product's relation slices, or `None` where the product is gone.
///
/// A vanished product is not a fault: an election row outliving its product
/// is a race with a delete, and the question still renders with its
/// candidates and without a suggestion, which is honest — there is no
/// resolved set to rank because there is no longer anything that resolved.
async fn resolved_context(
    state: &AppState,
    org: OrgId,
    key: (ProductId, InventoryId),
) -> Result<Option<ResolvedContext>, APIError> {
    let (product, inventory) = key;
    let taxonomy = TaxonomyRepo::new(state.pool.clone());
    let Some(record) = ProductRepo::new(state.pool.clone())
        .get(org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?
    else {
        return Ok(None);
    };
    let edges = taxonomy
        .edges_into_all(&projection_vocabularies(inventory, &record.product))
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let no_counterparts = taxonomy
        .no_counterparts_into(inventory)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(Some(ResolvedContext {
        product: record.product,
        edges,
        no_counterparts,
    }))
}

/// The losses of the mapping that raised the question. An election authored
/// on the create form names no mapping and so names no losses yet, which is
/// honest: nothing has been projected.
async fn losses_of(
    state: &AppState,
    mappings: &MappingRepo,
    org: OrgId,
    raised_by: Option<MappingId>,
) -> Result<Vec<LossView>, APIError> {
    let Some(mapping) = raised_by else {
        return Ok(Vec::new());
    };
    Ok(mappings
        .losses(org, mapping)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .into_iter()
        .map(|loss| LossView {
            kind: loss.kind.as_str().to_owned(),
            axis: loss.axis,
            detail: loss.detail,
            recorded_at: loss.recorded_at,
        })
        .collect())
}

#[derive(Debug, Deserialize)]
pub struct AnswerBody {
    /// One path per answered value: one for a supply or a primary pick,
    /// several for a band the seller narrows to more than one year group.
    pub answers: Vec<AnswerPath>,
    /// Whether this answer also becomes the tenant's standing rule, so every
    /// later product with the same question resolves without asking.
    ///
    /// Explicit and defaulted off: promotion is the seller saying "always",
    /// and inferring it from an answer's shape would turn one decision about
    /// one listing into a policy over every listing after it. It applies only
    /// to a question a rule can be keyed to — a supply keys on the pricing
    /// branch and a narrow on the source value's own native id, while an
    /// elect-one and an over-cap ask about one product's resolved set and
    /// generalise to nothing.
    #[serde(default)]
    pub apply_to_future: bool,
}

#[derive(Debug, Deserialize)]
pub struct AnswerPath {
    pub segments: Vec<String>,
    #[serde(default)]
    pub native_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AnswerAck {
    /// How many parked items the answer released, in the same transaction
    /// that recorded it.
    pub revived: u64,
    /// Whether a standing rule was written beside the answer. Reported rather
    /// than echoed back from the request: a seller who asked to apply the
    /// answer to a question no rule can be keyed to is told the answer stayed
    /// with this one product instead of being left to assume otherwise.
    pub promoted: bool,
}

pub(crate) async fn answer_decision(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, item)): Path<(String, String)>,
    Json(body): Json<AnswerBody>,
) -> Result<Json<AnswerAck>, APIError> {
    if body.answers.is_empty() {
        return Err(validation("an answered election names at least one value"));
    }
    let item_id = parse_id(&item)?;
    let repo = ElectionRepo::new(state.pool.clone());
    let open = repo
        .open_items(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let target = open
        .into_iter()
        .find(|candidate| candidate.id == item_id)
        .ok_or_else(|| missing("no such open election"))?;
    let vocabulary = VocabularyId(target.inventory, target.axis);
    let paths: Vec<VocabularyPath> = body
        .answers
        .into_iter()
        .map(|answer| VocabularyPath {
            vocabulary,
            segments: answer.segments,
            native_id: answer.native_id,
        })
        .collect();
    if paths.iter().any(|path| path.segments.is_empty()) {
        return Err(validation(
            "an answered value needs at least one path segment",
        ));
    }
    let now = (state.wall)();
    // Promotion is refused rather than silently keyless for the two triggers
    // that carry no key: a keyless rule is stored with the `''` sentinel and
    // would answer every future question on that axis from one product's own
    // resolved set. `election_rule_trigger_key` states the same rule at the
    // row, and this is the half that can say why.
    let promote = match (body.apply_to_future, target.trigger_key.clone()) {
        (true, Some(key)) => Some(standing_rule(&context, &target, key, &paths, now)?),
        (true, None) | (false, _) => None,
    };
    let report = repo
        .answer(
            context.org,
            NewAnswer {
                item: item_id,
                paths: &paths,
                at: now,
                promote: promote.as_ref(),
            },
        )
        .await
        .map_err(|error| conflict_or_fault(&state, &error))?;
    Ok(Json(AnswerAck {
        revived: report.revived,
        promoted: report.promoted,
    }))
}

/// The standing rule an answered election promotes to, built through
/// `ElectionRule::new` so the legal backstop refuses a delegated licence here
/// exactly as it does on the rule endpoint.
///
/// One named value is a `Value` and several are an `Ordering`, which is the
/// same reading `resolved_by` gives the stored answer itself: a band narrowed
/// to three year groups is a preference over three, not three separate rules.
fn standing_rule(
    context: &OrgContext,
    target: &OpenElection,
    trigger_key: String,
    paths: &[VocabularyPath],
    now: Timestamp,
) -> Result<ElectionRule, APIError> {
    let answer = match paths {
        [path] => ElectionAnswer::Value { path: path.clone() },
        several => ElectionAnswer::Ordering {
            prefer: several.to_vec(),
        },
    };
    ElectionRule::new(NewElectionRule {
        org: context.org,
        inventory: target.inventory,
        axis: target.axis,
        trigger_kind: target.trigger_kind,
        trigger_key: Some(trigger_key),
        answer,
        decided_by: Decider::Human {
            user: context.user,
            org: context.org,
        },
        decided_at: now,
    })
    .map_err(|error| match error {
        ElectionRuleError::NotDelegable(_) => validation(
            "this axis is the seller's own: choosing a rights grant is issuing one, \
             so no opt-in delegates it to a computation",
        ),
        ElectionRuleError::UnboundAxis => {
            validation("this inventory declares no such equivalence axis")
        }
    })
}

pub(crate) async fn withdraw_decision(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, item)): Path<(String, String)>,
) -> Result<StatusCode, APIError> {
    let item_id = parse_id(&item)?;
    ElectionRepo::new(state.pool.clone())
        .withdraw(context.org, item_id, (state.wall)())
        .await
        .map_err(|error| conflict_or_fault(&state, &error))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct RuleBody {
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub trigger: String,
    #[serde(default)]
    pub trigger_key: Option<String>,
    /// `value`, `ordering` or `delegate`. A legal axis refuses `delegate` at
    /// both layers: here through `ElectionRule::new`, and again at the row.
    pub answer_kind: String,
    #[serde(default)]
    pub answers: Vec<AnswerPath>,
}

pub(crate) async fn upsert_rule(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<RuleBody>,
) -> Result<StatusCode, APIError> {
    let trigger = ElectionTriggerKind::ALL
        .into_iter()
        .find(|kind| kind.as_str() == body.trigger)
        .ok_or_else(|| validation("unknown election trigger"))?;
    let vocabulary = VocabularyId(body.inventory, body.axis);
    let paths: Vec<VocabularyPath> = body
        .answers
        .into_iter()
        .map(|answer| VocabularyPath {
            vocabulary,
            segments: answer.segments,
            native_id: answer.native_id,
        })
        .collect();
    let answer = match body.answer_kind.as_str() {
        "value" => {
            let [path] = paths.as_slice() else {
                return Err(validation("a value rule names exactly one path"));
            };
            ElectionAnswer::Value { path: path.clone() }
        }
        "ordering" if !paths.is_empty() => ElectionAnswer::Ordering { prefer: paths },
        "ordering" => return Err(validation("an ordering rule names at least one path")),
        "delegate" => ElectionAnswer::Delegate,
        other => return Err(validation(&format!("unknown answer kind {other}"))),
    };
    let rule = ElectionRule::new(NewElectionRule {
        org: context.org,
        inventory: body.inventory,
        axis: body.axis,
        trigger_kind: trigger,
        trigger_key: body.trigger_key,
        answer,
        decided_by: Decider::Human {
            user: context.user,
            org: context.org,
        },
        decided_at: (state.wall)(),
    })
    .map_err(|error| match error {
        ElectionRuleError::NotDelegable(_) => validation(
            "this axis is the seller's own: choosing a rights grant is issuing one, \
             so no opt-in delegates it to a computation",
        ),
        ElectionRuleError::UnboundAxis => {
            validation("this inventory declares no such equivalence axis")
        }
    })?;
    ElectionRepo::new(state.pool.clone())
        .upsert_rule(&rule)
        .await
        .map_err(|error| conflict_or_fault(&state, &error))?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------- delegation

/// One axis of one marketplace, and where the seller stands on letting us
/// choose its value.
#[derive(Debug, Serialize, Deserialize)]
pub struct AxisDelegationView {
    pub inventory: InventoryId,
    pub axis: TermKind,
    /// The native field the axis lands in, so a tab can name what the tick
    /// covers in the platform's own words rather than in ours.
    pub native: String,
    /// Whether this tenant has handed the axis to best fit.
    pub delegated: bool,
    /// Whether it may be handed over at all, and why not where it may not.
    /// A `never` axis reads `delegated: false` however many rows exist,
    /// because two layers refuse to write one and `resolution_for` would
    /// ignore it if one appeared.
    pub delegation: crate::vocabulary::DelegationView,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DelegationsView {
    pub items: Vec<AxisDelegationView>,
}

/// Ticking or unticking best fit for one marketplace.
///
/// Per marketplace rather than per axis, because that is the control: one
/// checkbox at the head of a marketplace tab reading "choose the best fit for
/// me on this marketplace". Which axes it reaches is the registry's to say,
/// not the caller's, so the body names no axis and cannot be used to reach
/// one the registry protects.
#[derive(Debug, Deserialize)]
pub struct DelegationBody {
    pub inventory: InventoryId,
    pub delegated: bool,
}

/// The two triggers a blanket delegation can be stored under.
///
/// `election_rule_trigger_key` admits a keyless rule for exactly these two,
/// and a marketplace-level checkbox has no key to give: a supply keys on the
/// pricing branch and a narrow on one source value's own native id, and the
/// tick knows neither. Reading the opt-in back per `(inventory, axis)` rather
/// than per trigger is what still lets those two see the delegation.
///
/// Both are written rather than one, though the read-back would be satisfied
/// by either. The rows are the audit record, and a row on one of the two
/// would say the seller delegated elect-one and withheld over-cap, which is
/// not the question they were asked; the table's key makes "this axis is
/// delegated" expressible only as a row per admissible trigger. The untick
/// removes both.
const DELEGABLE_TRIGGERS: [ElectionTriggerKind; 2] =
    [ElectionTriggerKind::ElectOne, ElectionTriggerKind::OverCap];

/// Every axis of every inventory with the seller's own standing on it, so a
/// form can render the tick without inferring anything.
///
/// Total over the registry rather than over the rows: an axis with no rule is
/// not delegated, which is the same answer a tenant who has written none gets
/// today, and a `never` axis appears with its reason so the control can be
/// disabled with words rather than silently absent.
fn delegations_view(rules: &[ElectionRule]) -> DelegationsView {
    let delegated = delegated_axes(rules);
    let mut items = Vec::new();
    for inventory in InventoryId::ALL {
        for binding in registry(inventory).equivalence_axes {
            items.push(AxisDelegationView {
                inventory,
                axis: binding.axis,
                native: binding.native.to_owned(),
                delegated: matches!(resolution_for(*binding, true), Mode::BestFit)
                    && delegated.contains(&(inventory, binding.axis)),
                delegation: crate::vocabulary::DelegationView::of(binding.delegation),
            });
        }
    }
    DelegationsView { items }
}

pub(crate) async fn list_delegations(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<DelegationsView>, APIError> {
    let rules = ElectionRepo::new(state.pool.clone())
        .rules(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(delegations_view(&rules)))
}

/// The tick and the untick, as one durable and revocable fact per axis.
///
/// Ticking writes an `ElectionRule` per delegable axis rather than setting a
/// client-side preference, so the delegation is a row the seller can see,
/// audit and withdraw, and so the legal backstop refuses it in the two places
/// it already refuses everything else. It reaches no `Delegation::Never`
/// axis: `ElectionRule::new` refuses one and the table's own CHECK refuses it
/// again, and this filters them out before either has to, so a tick over a
/// marketplace carrying a licence axis succeeds on everything but the licence
/// instead of failing whole.
///
/// Unticking removes the delegations and nothing else, so a standing answer
/// the seller stated themselves survives a change of mind about best fit.
pub(crate) async fn set_delegation(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<DelegationBody>,
) -> Result<Json<DelegationsView>, APIError> {
    let repo = ElectionRepo::new(state.pool.clone());
    let now = (state.wall)();
    if body.delegated {
        let mut rules = Vec::new();
        for binding in registry(body.inventory).equivalence_axes {
            if resolution_for(*binding, true) != Mode::BestFit {
                continue;
            }
            for trigger_kind in DELEGABLE_TRIGGERS {
                rules.push(delegation_rule(
                    &context,
                    body.inventory,
                    *binding,
                    trigger_kind,
                    now,
                )?);
            }
        }
        repo.upsert_rules(&rules)
            .await
            .map_err(|error| conflict_or_fault(&state, &error))?;
    } else {
        repo.revoke_delegation(context.org, body.inventory)
            .await
            .map_err(|error| conflict_or_fault(&state, &error))?;
    }
    let rules = repo
        .rules(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(delegations_view(&rules)))
}

/// One delegation row, built through `ElectionRule::new` so this route is not
/// a second way in past the legal refusal.
fn delegation_rule(
    context: &OrgContext,
    inventory: InventoryId,
    binding: AxisBinding,
    trigger_kind: ElectionTriggerKind,
    now: Timestamp,
) -> Result<ElectionRule, APIError> {
    ElectionRule::new(NewElectionRule {
        org: context.org,
        inventory,
        axis: binding.axis,
        trigger_kind,
        trigger_key: None,
        answer: ElectionAnswer::Delegate,
        decided_by: Decider::Human {
            user: context.user,
            org: context.org,
        },
        decided_at: now,
    })
    .map_err(|error| match error {
        ElectionRuleError::NotDelegable(_) => validation(
            "this axis is the seller's own: choosing a rights grant is issuing one, \
             so no opt-in delegates it to a computation",
        ),
        ElectionRuleError::UnboundAxis => {
            validation("this inventory declares no such equivalence axis")
        }
    })
}

/// A storage refusal the seller caused, told apart from one they did not. The
/// repo states an unanswerable item as `Inconsistent`, which is a conflict
/// rather than a fault.
fn conflict_or_fault(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    if let tam_storage::StorageError::Inconsistent { reason } = error {
        APIError::new(
            StatusCode::CONFLICT,
            APIErrorEntry::new(reason).kind(APIErrorKind::Validation),
        )
    } else {
        storage_fault(state, error)
    }
}

#[cfg(test)]
mod tests {
    use super::{delegated_axes, delegations_view, rebuilt_trigger};
    use tam_domain::equivalence::{
        ElectionAnswer, ElectionRule, ElectionTrigger, ElectionTriggerKind, NewElectionRule,
    };
    use tam_domain::registry::registry;
    use tam_domain::{Decider, TermKind, VocabularyId, VocabularyPath};
    use tam_types::{InventoryId, OrgId, Timestamp, Uuid};

    const ORG: OrgId = OrgId(Uuid([0x01; 16]));

    fn axis(inventory: InventoryId, axis: TermKind) -> tam_domain::registry::AxisBinding {
        match registry(inventory).axis(axis) {
            Some(binding) => binding,
            None => panic!("{inventory:?} binds {axis:?}"),
        }
    }

    fn path(segment: &str) -> VocabularyPath {
        VocabularyPath {
            vocabulary: VocabularyId(InventoryId::Tpt, TermKind::Phase),
            segments: vec![segment.to_owned()],
            native_id: Some(segment.to_owned()),
        }
    }

    fn rule(answer: ElectionAnswer) -> ElectionRule {
        match ElectionRule::new(NewElectionRule {
            org: ORG,
            inventory: InventoryId::Tes,
            axis: TermKind::Subject,
            trigger_kind: ElectionTriggerKind::ElectOne,
            trigger_key: None,
            answer,
            decided_by: Decider::Imported {
                source: "test".to_owned(),
            },
            decided_at: Timestamp(0),
        }) {
            Ok(built) => built,
            Err(error) => panic!("the subject axis is delegable, got {error:?}"),
        }
    }

    /// The cap that raised an over-cap question is the registry's own, so the
    /// rebuild reads it from there rather than from the row. TPT's phase axis
    /// is the one axis anywhere that declares one.
    #[test]
    fn an_over_cap_rebuild_takes_the_cap_the_registry_declares() {
        let resolved = vec![path("1"), path("2"), path("3"), path("4"), path("5")];
        assert_eq!(
            rebuilt_trigger(
                ElectionTriggerKind::OverCap,
                axis(InventoryId::Tpt, TermKind::Phase),
                resolved.clone()
            ),
            Some(ElectionTrigger::OverCap {
                cap: 4,
                from: resolved.clone()
            }),
            "TPT's create form says four grades, and that is the number the suggestion keeps"
        );
        assert_eq!(
            rebuilt_trigger(
                ElectionTriggerKind::OverCap,
                axis(InventoryId::Tpt, TermKind::Subject),
                resolved
            ),
            None,
            "an axis declaring no cap cannot be asked what it would keep, because an absent \
             cap is unmeasured rather than unlimited"
        );
    }

    /// The two the durable row cannot answer, refused rather than ranked
    /// against a set that is not the one the question was asked about.
    #[test]
    fn a_supply_and_a_narrow_are_not_rebuilt_from_the_row() {
        for kind in [ElectionTriggerKind::Supply, ElectionTriggerKind::Narrow] {
            assert_eq!(
                rebuilt_trigger(
                    kind,
                    axis(InventoryId::Tes, TermKind::Subject),
                    vec![path("1")]
                ),
                None,
                "{kind:?} carries a question this row cannot reconstruct the candidates for"
            );
        }
        assert_eq!(
            rebuilt_trigger(
                ElectionTriggerKind::ElectOne,
                axis(InventoryId::Tes, TermKind::ResourceType),
                vec![path("1")]
            ),
            Some(ElectionTrigger::ElectOne {
                from: vec![path("1")]
            }),
            "an elect-one asks about this product's own resolved set, which is exactly what \
             the route recomputes"
        );
    }

    /// The opt-in is a delegation and not any standing answer: a seller who
    /// stated a literal value has decided rather than delegated, and reading
    /// their decision as an opt-in would compute a suggestion they never
    /// asked for.
    #[test]
    fn only_a_delegate_answer_counts_as_an_opt_in() {
        assert!(
            delegated_axes(&[
                rule(ElectionAnswer::Value { path: path("1") }),
                rule(ElectionAnswer::Ordering {
                    prefer: vec![path("1")]
                }),
            ])
            .is_empty(),
            "a stated answer is a decision, not a permission"
        );
        assert_eq!(
            delegated_axes(&[rule(ElectionAnswer::Delegate)])
                .into_iter()
                .collect::<Vec<_>>(),
            vec![(InventoryId::Tes, TermKind::Subject)],
            "the delegation is keyed on the marketplace and the axis, not on the trigger"
        );
    }

    /// The view is total over the registry rather than over the rows, and a
    /// legal axis reads undelegated whatever exists, so a form can render the
    /// control disabled with its reason rather than omitting it.
    #[test]
    fn the_delegation_view_is_total_and_a_legal_axis_never_reads_delegated() {
        let view = delegations_view(&[]);
        let tes: Vec<TermKind> = view
            .items
            .iter()
            .filter(|item| item.inventory == InventoryId::Tes)
            .map(|item| item.axis)
            .collect();
        assert_eq!(
            tes,
            vec![
                TermKind::Subject,
                TermKind::Topic,
                TermKind::ResourceType,
                TermKind::Phase,
                TermKind::Licence,
            ],
            "every axis the marketplace binds appears, whether or not a rule exists for it"
        );
        assert!(
            view.items.iter().all(|item| !item.delegated),
            "a tenant with no rules is opted into nothing, which is the shipped behaviour"
        );
        let licence = view
            .items
            .iter()
            .find(|item| item.inventory == InventoryId::Tes && item.axis == TermKind::Licence);
        assert_eq!(
            licence.map(|item| (item.native.as_str(), item.delegation.kind)),
            Some(("licence", crate::vocabulary::DelegationKind::Never)),
            "the tick names what it does not cover, in the field's own wire name"
        );
    }
}

#[cfg(test)]
mod image_tests {
    use super::image_type;
    use tam_pipeline::probe::probe_kind;
    use tam_types::FileKind;

    /// Every byte string this crate's fixtures cover, so the two functions
    /// below are compared over one set rather than over two that could drift.
    const FIXTURES: [&[u8]; 12] = [
        b"\x89PNG\r\n\x1a\nrest",
        b"\xFF\xD8\xFF\xE0\x00\x10JFIF",
        b"GIF87a\x08\x00",
        b"GIF89a\x08\x00",
        b"GIF8ZZ\x08\x00",
        b"RIFF\x24\x00\x00\x00WEBPVP8 ",
        b"RIFF\x24\x00\x00\x00WAVEfmt ",
        b"RIFF\x24\x00",
        b"%PDF-1.7 a worksheet",
        b"PK\x03\x04",
        b"<svg xmlns=\"http://www.w3.org/2000/svg\">",
        b"",
    ];

    /// Anything the upload admits as an image is one this route can name.
    ///
    /// This is the direction the write-side slot check rests on: a slot that
    /// stored bytes [`probe_kind`] called an image and [`image_type`] then
    /// refuses is a slot that reads back as 415 and draws nothing, which is a
    /// filled slot showing an empty tile. The converse deliberately does not
    /// hold — WebP is servable and not uploadable — so it is not asserted.
    #[test]
    fn every_kind_the_upload_calls_an_image_is_one_this_route_can_name() {
        let admitted = FIXTURES
            .into_iter()
            .filter(|bytes| probe_kind(bytes) == Some(FileKind::Image))
            .count();
        assert_eq!(
            admitted, 4,
            "the fixtures cover every format the upload admits as an image; a set that \
             matched none would make the implication below vacuous"
        );
        for bytes in FIXTURES {
            if probe_kind(bytes) == Some(FileKind::Image) {
                assert!(
                    image_type(bytes).is_some(),
                    "the upload admitted bytes this route cannot name: {bytes:?}"
                );
            }
        }
    }

    /// Every format the two byte-serving routes will name, decided from the
    /// bytes. A leading fragment is enough: the signature is what is read, and
    /// a fixture that decoded would prove the decoder rather than this.
    #[test]
    fn each_served_format_is_named_from_its_own_signature() {
        assert_eq!(image_type(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(
            image_type(b"\xFF\xD8\xFF\xE0\x00\x10JFIF"),
            Some("image/jpeg")
        );
        assert_eq!(image_type(b"GIF87a\x08\x00"), Some("image/gif"));
        assert_eq!(image_type(b"GIF89a\x08\x00"), Some("image/gif"));
        assert_eq!(
            image_type(b"RIFF\x24\x00\x00\x00WEBPVP8 "),
            Some("image/webp")
        );
    }

    /// Anything else is refused rather than served under a type nobody has to
    /// believe. The seller's own payload is the case that matters: a handle
    /// names a PDF as readily as a thumbnail.
    #[test]
    fn everything_else_is_refused() {
        assert_eq!(image_type(b"%PDF-1.7 a worksheet"), None);
        assert_eq!(image_type(b"PK\x03\x04"), None);
        assert_eq!(image_type(b""), None);
        assert_eq!(
            image_type(b"<svg xmlns=\"http://www.w3.org/2000/svg\">"),
            None,
            "an SVG executes script in the browser, so it is a document rather than a picture"
        );
    }

    /// A RIFF container that is not WEBP is not an image, and the form type is
    /// what separates them: WAV and AVI carry the same first four bytes.
    #[test]
    fn a_riff_container_that_is_not_webp_is_refused() {
        assert_eq!(image_type(b"RIFF\x24\x00\x00\x00WAVEfmt "), None);
        assert_eq!(
            image_type(b"RIFF\x24\x00"),
            None,
            "and a truncated one is refused rather than read past its end"
        );
    }
}
