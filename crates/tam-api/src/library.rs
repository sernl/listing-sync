//! The seller's own files, on the seller's own machines: which machine holds
//! which, and where one can reach another to take a copy directly.
//!
//! The server coordinates and carries nothing. A device reports, on its
//! heartbeat, the node address its endpoint listens on and the digests it
//! holds; the console asks one device to fetch one file; that device asks
//! here which of the seller's other devices hold it and where they are, and
//! connects to one of them itself. No route in this module accepts or serves
//! a byte of a file, and none could: the storage it reads has no column for
//! one.
//!
//! There is no relay. Two devices that cannot reach each other directly do
//! not transfer, and the console says the file is waiting for the other
//! machine rather than routing it through us.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    DeviceLibraryRepo, HoldingReport, LibraryAvailability, LibraryFilter, LibraryLinked,
    LibraryReport, LIBRARY_LIMIT_DEFAULT, LIBRARY_LIMIT_MAX,
};
use tam_types::{ContentHash, OrgId, Timestamp};

use crate::devices::{HeartbeatLibrary, ID_MAX_CHARS};
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
        _ => Err(validation(
            "availability is one of online, offline or missing",
        )),
    }
}

fn linked_of(raw: &str) -> Result<LibraryLinked, APIError> {
    match raw {
        "linked" => Ok(LibraryLinked::Linked),
        "unlinked" => Ok(LibraryLinked::Unlinked),
        _ => Err(validation("linked is one of linked or unlinked")),
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
        return Err(validation("a search is at most 128 characters"));
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
    Ok(Json(LibraryView {
        files: page
            .files
            .into_iter()
            .map(|file| LibraryFileView {
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
        return Err(missing("no other machine of yours holds that file"));
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
