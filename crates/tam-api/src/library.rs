//! The seller's own files, on the seller's own machines: which machine holds
//! which, where one can reach another to take a copy directly, and
//! Teachouse's own copy of each.
//!
//! Between devices the server coordinates and carries nothing. A device
//! reports, on its heartbeat, the node address its endpoint listens on and
//! the digests it holds; the console asks one device to fetch one file; that
//! device asks here which of the seller's other devices hold it and where
//! they are, and connects to one of them itself. There is no relay: two
//! devices that cannot reach each other directly do not transfer.
//!
//! The server copy is the one place bytes arrive. An import leaves the
//! original on the device that read it; that device's app then copies each
//! one here (`GET /library/missing`, `PUT /library/files/{hash}`), so the
//! seller can view, cut a preview from and download the file wherever they
//! sign in. The copy is a blob like any upload, charged against the plan's
//! storage, and refused rather than taken where the plan has no room.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    BlobRepo, DeviceLibraryRepo, HoldingReport, LibraryAvailability, LibraryFilter, LibraryLinked,
    LibraryReport, ProductRepo, ServerCopyRepo, LIBRARY_LIMIT_DEFAULT, LIBRARY_LIMIT_MAX,
};
use tam_types::{ContentHash, OrgId, Timestamp};

use crate::devices::{HeartbeatLibrary, ID_MAX_CHARS};
use crate::entitlement::{quota_refusal, QuotaKind};
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

/// How recently a device must have checked in to count as online for a
/// transfer. Ten minutes: two ordinary check-ins, so one missed beat does
/// not read as a machine that went away.
pub const ONLINE_WINDOW_MS: i64 = 10 * 60 * 1_000;

/// The bound on a node id and a direct address, so a device cannot file
/// arbitrarily long strings under the organisation.
const NODE_ID_MAX_CHARS: usize = 128;
const ADDR_MAX_CHARS: usize = 128;
const ADDRS_MAX: usize = 16;
const HOLDINGS_MAX: usize = 10_000;

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

/// A digest as the wire spells it: sixty-four lowercase hex characters.
pub(crate) fn hash_of(hex: &str) -> Result<ContentHash, APIError> {
    if hex.len() != 64 {
        return Err(validation("a file is named by its 64-character hex digest"));
    }
    let mut bytes = [0u8; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        let pair = core::str::from_utf8(pair)
            .map_err(|_| validation("a file is named by its hex digest"))?;
        bytes[index] = u8::from_str_radix(pair, 16)
            .map_err(|_| validation("a file is named by its hex digest"))?;
    }
    Ok(ContentHash(bytes))
}

fn hex_of(hash: ContentHash) -> String {
    tam_secrets::hex_encode(&hash.0)
}

/// Records what a heartbeat said about the device's library.
pub(crate) async fn record_report(
    state: &AppState,
    org: OrgId,
    device: &str,
    library: &HeartbeatLibrary,
    now: Timestamp,
) -> Result<(), APIError> {
    if library.node_id.is_empty() || library.node_id.chars().count() > NODE_ID_MAX_CHARS {
        return Err(validation("a node id is between 1 and 128 characters"));
    }
    if library.direct_addrs.len() > ADDRS_MAX
        || library
            .direct_addrs
            .iter()
            .any(|addr| addr.is_empty() || addr.chars().count() > ADDR_MAX_CHARS)
    {
        return Err(validation(
            "a device reports at most sixteen direct addresses of up to 128 characters",
        ));
    }
    if library.holdings.len() > HOLDINGS_MAX {
        return Err(validation("a device reports at most ten thousand holdings"));
    }
    let holdings = library
        .holdings
        .iter()
        .map(|holding| {
            Ok(HoldingReport {
                hash: hash_of(&holding.hash)?,
                byte_len: holding.byte_len,
            })
        })
        .collect::<Result<Vec<_>, APIError>>()?;
    DeviceLibraryRepo::new(state.pool.clone())
        .report(
            org,
            device,
            &LibraryReport {
                node_id: &library.node_id,
                direct_addrs: &library.direct_addrs,
                holdings: &holdings,
            },
            now,
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HolderView {
    pub device: String,
    pub name: String,
    /// Checked in within [`ONLINE_WINDOW_MS`] of the read.
    pub online: bool,
}

/// One resource a file belongs to, as the browser links it: the identifier
/// `/resources/{id}` takes, and the title it shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryResourceView {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryFileView {
    pub hash: String,
    pub file_name: Option<String>,
    pub byte_len: u64,
    pub holders: Vec<HolderView>,
    pub wanted_by: Vec<String>,
    /// Every live resource of this seller's that uses these bytes. Empty
    /// where a machine keeps a file no resource uses, and where the only
    /// resource that used it was deleted.
    pub resources: Vec<LibraryResourceView>,
    /// Whether Teachouse holds a copy, and why not where it does not.
    pub server_copy: ServerCopy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryView {
    pub files: Vec<LibraryFileView>,
    /// Files matching the filter, not files on the page: the pager needs
    /// the figure the page was cut from.
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
}

/// The bound on the search box, so a filter cannot be a payload.
const SEARCH_MAX_CHARS: usize = 128;

/// What the file browser asked for. Every member is optional; the bare
/// route answers the first page of everything, as it did before there was
/// a browser to ask.
#[derive(Debug, Deserialize)]
pub struct LibraryParams {
    pub q: Option<String>,
    pub device: Option<String>,
    pub availability: Option<String>,
    pub linked: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

fn availability_of(raw: &str) -> Result<LibraryAvailability, APIError> {
    match raw {
        "online" => Ok(LibraryAvailability::Online),
        "offline" => Ok(LibraryAvailability::Offline),
        "missing" => Ok(LibraryAvailability::Missing),
        _ => Err(validation("Choose online, offline or missing.")),
    }
}

fn linked_of(raw: &str) -> Result<LibraryLinked, APIError> {
    match raw {
        "linked" => Ok(LibraryLinked::Linked),
        "unlinked" => Ok(LibraryLinked::Unlinked),
        _ => Err(validation("Choose linked or unlinked.")),
    }
}

/// A search term the seller left blank is no search at all, which is what
/// an empty box posts: the alternative is a filter matching every file by
/// the empty string and a total nobody can account for.
fn search_of(raw: Option<&String>) -> Result<Option<&str>, APIError> {
    let Some(term) = raw.map(|term| term.trim()).filter(|term| !term.is_empty()) else {
        return Ok(None);
    };
    if term.chars().count() > SEARCH_MAX_CHARS {
        return Err(validation("Keep your search to 128 characters or fewer."));
    }
    Ok(Some(term))
}

pub(crate) async fn list_library(
    State(state): State<AppState>,
    context: OrgContext,
    Query(params): Query<LibraryParams>,
) -> Result<Json<LibraryView>, APIError> {
    let now = (state.wall)();
    let limit = params.limit.unwrap_or(LIBRARY_LIMIT_DEFAULT);
    if limit == 0 || limit > LIBRARY_LIMIT_MAX {
        return Err(validation("a page holds between 1 and 100 files"));
    }
    let device = params.device.as_deref().map(device_of).transpose()?;
    let filter = LibraryFilter {
        search: search_of(params.q.as_ref())?,
        device,
        availability: params
            .availability
            .as_deref()
            .map(availability_of)
            .transpose()?,
        linked: params.linked.as_deref().map(linked_of).transpose()?,
        offset: params.offset.unwrap_or(0),
        limit,
        online_after: Timestamp(now.0.saturating_sub(ONLINE_WINDOW_MS)),
    };
    let page = DeviceLibraryRepo::new(state.pool.clone())
        .page(context.org, &filter)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let hashes: Vec<ContentHash> = page.files.iter().map(|file| file.hash).collect();
    let ledger = CopyLedger::read(&state, &context, &hashes).await?;
    Ok(Json(LibraryView {
        files: page
            .files
            .into_iter()
            .map(|file| LibraryFileView {
                server_copy: ledger.state(file.hash, file.byte_len),
                hash: hex_of(file.hash),
                file_name: file.file_name,
                byte_len: file.byte_len,
                holders: file
                    .holders
                    .into_iter()
                    .map(|holder| HolderView {
                        online: now.0.saturating_sub(holder.last_seen_at.0) <= ONLINE_WINDOW_MS,
                        device: holder.device,
                        name: holder.name,
                    })
                    .collect(),
                wanted_by: file.wanted_by,
                resources: file
                    .resources
                    .into_iter()
                    .map(|resource| LibraryResourceView {
                        id: resource.id.to_hyphenated(),
                        title: resource.title,
                    })
                    .collect(),
            })
            .collect(),
        total: page.total,
        offset: page.offset,
        limit: page.limit,
    }))
}

#[derive(Debug, Deserialize)]
pub struct WantBody {
    pub hash: String,
}

fn device_of(raw: &str) -> Result<&str, APIError> {
    if raw.is_empty() || raw.chars().count() > ID_MAX_CHARS {
        return Err(validation("a device id is between 1 and 64 characters"));
    }
    Ok(raw)
}

/// The seller asks one device to fetch one file from another. Refused where
/// no live device of theirs holds it, because nothing could ever satisfy it.
pub(crate) async fn want(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
    Json(body): Json<WantBody>,
) -> Result<StatusCode, APIError> {
    let device = device_of(&device)?;
    let hash = hash_of(&body.hash)?;
    let repo = DeviceLibraryRepo::new(state.pool.clone());
    let held = repo
        .held_elsewhere(context.org, device, hash)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    if !held {
        return Err(missing("None of your other devices has that file."));
    }
    repo.want(context.org, device, hash, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn unwant(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
    Json(body): Json<WantBody>,
) -> Result<StatusCode, APIError> {
    let device = device_of(&device)?;
    DeviceLibraryRepo::new(state.pool.clone())
        .unwant(context.org, device, hash_of(&body.hash)?)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WantsView {
    pub hashes: Vec<String>,
}

/// What one device has been asked to fetch. Read by the device on its cycle.
pub(crate) async fn wants(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
) -> Result<Json<WantsView>, APIError> {
    let device = device_of(&device)?;
    let hashes = DeviceLibraryRepo::new(state.pool.clone())
        .wants_of(context.org, device)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(WantsView {
        hashes: hashes.into_iter().map(hex_of).collect(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct PeersParams {
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerView {
    pub device: String,
    pub node_id: String,
    pub direct_addrs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeersView {
    pub peers: Vec<PeerView>,
    /// Every node id the organisation's live devices report, which is the
    /// set the asking device accepts a connection from and offers to.
    pub trusted: Vec<String>,
}

/// Where the online holders of one file can be reached, for the device
/// that wants it.
pub(crate) async fn peers(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
    Query(params): Query<PeersParams>,
) -> Result<Json<PeersView>, APIError> {
    let device = device_of(&device)?;
    let hash = hash_of(&params.hash)?;
    let now = (state.wall)();
    let repo = DeviceLibraryRepo::new(state.pool.clone());
    let peers = repo
        .peers(
            context.org,
            device,
            hash,
            Timestamp(now.0.saturating_sub(ONLINE_WINDOW_MS)),
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let trusted = repo
        .node_ids(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(PeersView {
        peers: peers
            .into_iter()
            .map(|peer| PeerView {
                device: peer.device,
                node_id: peer.node_id,
                direct_addrs: peer.direct_addrs,
            })
            .collect(),
        trusted,
    }))
}

// ------------------------------------------------------------ server copies

/// The largest file the seller's app copies here: the upload's own ceiling,
/// in one request.
///
/// Not raised and not chunked, deliberately. Every request reaches us
/// through Cloudflare, which refuses a body over 100 MB at the edge on this
/// plan, so a larger single request cannot arrive; and a sealed blob is one
/// envelope sealed whole, so a chunked copy would still be assembled and
/// sealed in memory at about three times its size in a 256 MiB pod
/// (`docs/notes/design/research/2026-09-26-pricing-evidence-capacity.md`).
/// The founder's largest TPT product on 2026-09-19 was 60 MB. A file above
/// this stays on the device, and the console says so rather than the device
/// retrying a copy that cannot land.
pub const COPY_BYTES_MAX: u64 = tam_limits::http::UPLOAD_BODY_BYTES_MAX;

/// The largest file this server opens to serve. A sealed blob opens only
/// whole, so serving one holds all of it in memory for the request; above
/// this the read is refused rather than attempted.
pub const OPEN_BYTES_MAX: u64 = 256 * 1024 * 1024;

/// Where a file's bytes are, as the console reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerCopy {
    /// Teachouse holds the bytes: they can be viewed, cut into a preview and
    /// downloaded from any browser.
    Stored,
    /// Only the seller's device holds them, and its app will copy them here
    /// the next time it runs.
    DeviceOnly,
    /// Only the seller's device holds them, and copying them here would take
    /// the plan past its storage.
    StorageFull,
    /// Only the seller's device holds them, and they are larger than
    /// [`COPY_BYTES_MAX`].
    TooLarge,
}

/// What decides a file's [`ServerCopy`]: which digests are held, and how
/// much room the plan has left.
pub(crate) struct CopyLedger {
    held: std::collections::HashSet<ContentHash>,
    stored: u64,
    cap: u64,
}

impl CopyLedger {
    /// One read of the blobs among `hashes` and one of the tenant's total.
    pub(crate) async fn read(
        state: &AppState,
        context: &OrgContext,
        hashes: &[ContentHash],
    ) -> Result<Self, APIError> {
        let held = ServerCopyRepo::new(state.pool.clone())
            .held(context.org, hashes)
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
        let stored = ProductRepo::new(state.pool.clone())
            .stored_bytes(context.org)
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
        Ok(Self {
            held,
            stored: u64::try_from(stored).unwrap_or(0),
            cap: context.entitlement.caps.storage_bytes_max,
        })
    }

    pub(crate) fn state(&self, hash: ContentHash, byte_len: u64) -> ServerCopy {
        if self.held.contains(&hash) {
            ServerCopy::Stored
        } else if byte_len > COPY_BYTES_MAX {
            ServerCopy::TooLarge
        } else if self.stored.saturating_add(byte_len) > self.cap {
            ServerCopy::StorageFull
        } else {
            ServerCopy::DeviceOnly
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingFileView {
    pub hash: String,
    pub byte_len: u64,
}

/// The imported files Teachouse has no copy of, and the room there is for
/// them. Read by the seller's app, which copies the ones it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingView {
    /// Smallest first.
    pub files: Vec<MissingFileView>,
    pub stored_bytes: u64,
    pub storage_bytes_max: u64,
    /// The largest single copy [`copy_file`] accepts.
    pub copy_bytes_max: u64,
}

pub(crate) async fn missing_copies(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<MissingView>, APIError> {
    let files = ServerCopyRepo::new(state.pool.clone())
        .missing(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let stored = ProductRepo::new(state.pool.clone())
        .stored_bytes(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(MissingView {
        files: files
            .into_iter()
            .map(|file| MissingFileView {
                hash: hex_of(file.hash),
                byte_len: file.byte_len,
            })
            .collect(),
        stored_bytes: u64::try_from(stored).unwrap_or(0),
        storage_bytes_max: context.entitlement.caps.storage_bytes_max,
        copy_bytes_max: COPY_BYTES_MAX,
    }))
}

/// The tenant's storage after a copy, for the app to plan the next one
/// against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopiedView {
    pub stored_bytes: u64,
    pub storage_bytes_max: u64,
}

/// The body limit on [`copy_file`], one byte above the ceiling so the
/// handler, not the extractor, refuses a file that is exactly too large.
pub fn copy_body_limit() -> axum::extract::DefaultBodyLimit {
    axum::extract::DefaultBodyLimit::max(
        usize::try_from(COPY_BYTES_MAX.saturating_add(1)).unwrap_or(usize::MAX),
    )
}

fn blob_store_unavailable() -> APIError {
    APIError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        APIErrorEntry::new("File storage isn't available right now. Try again later.")
            .code(APIErrorCode::BlobStoreUnavailable)
            .kind(APIErrorKind::Internal),
    )
}

/// Takes the seller's app's copy of one imported file.
///
/// Only bytes an import already described: the digest must be one a live
/// resource's imported file names, so this is not a second upload route
/// without the upload's checks. The digest is recomputed over the body, the
/// plan's storage is checked before anything is sealed, and the bytes are
/// scanned here too, because from now on they are served in our voice.
///
/// Idempotent: a copy that is already held answers `200` with nothing
/// written, which is what a device retrying after a lost answer meets.
pub(crate) async fn copy_file(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, hash)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<CopiedView>), APIError> {
    let hash = hash_of(&hash)?;
    let blobs = state.blobs.clone().ok_or_else(blob_store_unavailable)?;
    let copies = ServerCopyRepo::new(state.pool.clone());
    let products = ProductRepo::new(state.pool.clone());
    let cap = context.entitlement.caps.storage_bytes_max;
    let stored_now = |stored: i64| CopiedView {
        stored_bytes: u64::try_from(stored).unwrap_or(0),
        storage_bytes_max: cap,
    };
    if copies
        .sourced_len(context.org, hash)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .is_none()
    {
        return Err(missing("None of your resources uses that file."));
    }
    if !copies
        .held(context.org, &[hash])
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .is_empty()
    {
        let stored = products
            .stored_bytes(context.org)
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
        return Ok((StatusCode::OK, Json(stored_now(stored))));
    }
    let incoming = u64::try_from(body.len()).unwrap_or(u64::MAX);
    if incoming > COPY_BYTES_MAX {
        return Err(APIError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            APIErrorEntry::new(
                "This file is too big to copy to Teachouse, so it stays on your device.",
            )
            .code(APIErrorCode::UploadRejected)
            .kind(APIErrorKind::Validation),
        ));
    }
    if tam_pipeline::hash::content_hash(&body) != hash {
        return Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new("Those bytes are not the file they were sent as.")
                .code(APIErrorCode::UploadRejected)
                .kind(APIErrorKind::Validation),
        ));
    }
    let used = products
        .stored_bytes(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let used_bytes = u64::try_from(used).unwrap_or(0);
    if used_bytes.saturating_add(incoming) > cap {
        return Err(quota_refusal(QuotaKind::StorageBytes, used, cap));
    }
    let now = (state.wall)();
    if let tam_types::ScanOutcome::Infected { signature } =
        tam_pipeline::scan::Scanner::scan(&tam_pipeline::scan::EicarScanner, &body, now).await
    {
        return Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new(&format!("This file did not pass a scan: {signature}"))
                .code(APIErrorCode::UploadRejected)
                .kind(APIErrorKind::Validation),
        ));
    }
    BlobRepo::new(state.pool.clone(), blobs.object_store(), blobs.kek.clone())
        .put(context.org, &body, now)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let stored = products
        .stored_bytes(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok((StatusCode::CREATED, Json(stored_now(stored))))
}

/// The bytes of one of the seller's files by digest, for the file browser,
/// which names files by digest rather than by resource.
///
/// Fenced like the resource route: the digest must be one a live resource of
/// this organisation uses, and the blob is read under the organisation.
pub(crate) async fn file_content(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, hash)): Path<(String, String)>,
    Query(query): Query<crate::resources::ContentQuery>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, APIError> {
    let hash = hash_of(&hash)?;
    let named = ServerCopyRepo::new(state.pool.clone())
        .named(context.org, hash)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .ok_or_else(|| missing("We can't find that file."))?;
    let Some(stored_len) = named.stored_len else {
        return Err(missing(crate::resources::DEVICE_ONLY));
    };
    let bytes = crate::resources::open_copy(&state, context.org, hash, stored_len)
        .await?
        .ok_or_else(|| missing(crate::resources::DEVICE_ONLY))?;
    let name = named
        .file_name
        .unwrap_or_else(|| format!("{}.{}", hex_of(hash), "bin"));
    crate::resources::file_answer(
        &state,
        bytes,
        &crate::resources::Served {
            name: &name,
            content_type: &named.content_type,
            download: query.wants_download(),
        },
        headers.get(axum::http::header::RANGE),
    )
}
