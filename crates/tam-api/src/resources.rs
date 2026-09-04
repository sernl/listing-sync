//! The catalogue, connections and reconciliation surfaces: what M1i's
//! client renders and the founder's drain workflow drives. Connection
//! revocation travels through the broker's unix socket — the only role that
//! can tombstone the vault — and a deployment without the socket answers
//! 503 rather than pretending to revoke.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_domain::equivalence::{
    resolution_for, ElectionAnswer, ElectionRule, ElectionRuleError, ElectionTriggerKind, Mode,
    NewElectionRule, NewProjectionOverride, OverrideKind, ProjectionOverride,
    ProjectionOverrideError,
};
use tam_domain::registry::listing_url::{listing_url, parse_listing_url, UrlRefusal};
use tam_domain::registry::registry;
use tam_domain::{Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId, VocabularyPath};
use tam_storage::{
    ConnectionFactsRepo, ConnectionRepo, DrainStats, ElectionRepo, LabelRepo, LedgerCursor,
    MappingRepo, NewAnswer, OpenElection, OverrideRepo, PastedBind, ProductRepo, StorageError,
    TaxonomyRepo,
};
use tam_taxonomy::check_native_ids;
use tam_types::{
    CanonicalTermId, ConnectionId, ConnectionStatus, CopyFormat, InventoryId, MappingId,
    Marketplace, OrgId, PriceIntent, ProductId, ScanOutcome, Timestamp, TransportClass, Uuid,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::jobs::{decode_cursor, encode_cursor};
use crate::{AppState, OrgContext};

fn storage_fault(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    state.internal(&error.to_string())
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
    Ok(Json(ProductsPage {
        products: rows
            .into_iter()
            .map(|row| ProductHead {
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
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileView {
    pub id: Uuid,
    pub role: String,
    pub kind: String,
    pub byte_len: u64,
    pub scan: String,
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
    let aggregate = record.product;
    let mut files: Vec<FileView> = aggregate
        .payload
        .iter()
        .map(|file| FileView {
            id: file.id.0,
            role: role_str(file.role).to_owned(),
            kind: kind_str(file.kind).to_owned(),
            byte_len: file.byte_len,
            scan: scan_str(&file.scan).to_owned(),
        })
        .collect();
    if let Some(cover) = &aggregate.cover {
        files.push(FileView {
            id: cover.id.0,
            role: role_str(cover.role).to_owned(),
            kind: kind_str(cover.kind).to_owned(),
            byte_len: cover.byte_len,
            scan: scan_str(&cover.scan).to_owned(),
        });
    }
    for preview in &aggregate.previews {
        files.push(FileView {
            id: preview.id.0,
            role: role_str(preview.role).to_owned(),
            kind: kind_str(preview.kind).to_owned(),
            byte_len: preview.byte_len,
            scan: scan_str(&preview.scan).to_owned(),
        });
    }
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
            })
            .collect(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RevokedView {
    pub connections: u32,
    pub elapsed_ms: i64,
}

/// The wire request the broker's protocol module defines; re-encoded here
/// because that module is deliberately private to the privilege boundary.
#[derive(Serialize)]
struct BrokerRevoke<'a> {
    op: &'a str,
    org: OrgId,
    connection: ConnectionId,
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum BrokerAnswer {
    Revoked {
        connections: u32,
        elapsed_ms: i64,
    },
    Error {
        detail: String,
        #[serde(default)]
        code: Option<BrokerErrorCode>,
    },
    #[serde(other)]
    Unexpected,
}

/// The machine-readable half of a broker error, mirrored from the broker's
/// own protocol module because that module is private to the privilege
/// boundary.
///
/// `Unrecognised` is the point of the type: a broker newer than this build
/// must be able to name a fault this build does not know, and the answer to
/// one is the internal path rather than a deserialisation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BrokerErrorCode {
    PlatformAccountAlreadyLinked,
    #[serde(other)]
    Unrecognised,
}

/// The API code a broker fault surfaces as.
///
/// Only faults the seller can act on cross as themselves; everything else
/// stays internal, because the broker's own words describe a privilege
/// boundary the client has no business reading.
fn broker_fault(state: &AppState, detail: &str, code: Option<BrokerErrorCode>) -> APIError {
    match code {
        Some(BrokerErrorCode::PlatformAccountAlreadyLinked) => APIError::new(
            StatusCode::CONFLICT,
            APIErrorEntry::new(detail)
                .code(APIErrorCode::PlatformAccountAlreadyLinked)
                .kind(APIErrorKind::Validation),
        ),
        Some(BrokerErrorCode::Unrecognised) | None => state.internal(detail),
    }
}

pub(crate) async fn revoke_connection(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, connection)): Path<(String, String)>,
) -> Result<Json<RevokedView>, APIError> {
    let connection = ConnectionId(parse_id(&connection)?);
    let Some(socket) = state.config.broker_socket.clone() else {
        return Err(APIError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            APIErrorEntry::new("the credential broker is not configured; nothing was revoked")
                .code(APIErrorCode::BrokerUnavailable)
                .kind(APIErrorKind::Internal),
        ));
    };
    let request = serde_json::to_string(&BrokerRevoke {
        op: "revoke",
        org: context.org,
        connection,
    })
    .map_err(|error| state.internal(&error.to_string()))?;

    let stream = tokio::net::UnixStream::connect(&socket)
        .await
        .map_err(|error| state.internal(&format!("broker socket: {error}")))?;
    let (read_half, mut write_half) = stream.into_split();
    write_half
        .write_all(format!("{request}\n").as_bytes())
        .await
        .map_err(|error| state.internal(&format!("broker write: {error}")))?;
    let mut line = String::new();
    BufReader::new(read_half)
        .read_line(&mut line)
        .await
        .map_err(|error| state.internal(&format!("broker read: {error}")))?;
    match serde_json::from_str(&line) {
        Ok(BrokerAnswer::Revoked {
            connections,
            elapsed_ms,
        }) => Ok(Json(RevokedView {
            connections,
            elapsed_ms,
        })),
        Ok(BrokerAnswer::Error { detail, code }) => Err(broker_fault(&state, &detail, code)),
        Ok(BrokerAnswer::Unexpected) | Err(_) => {
            Err(state.internal("the broker answered something unexpected"))
        }
    }
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
            ))
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
const LABELS_PER_PRODUCT_MAX: usize = 20;

fn labels_view(records: Vec<tam_storage::LabelRecord>) -> Json<LabelsView> {
    Json(LabelsView {
        labels: records
            .into_iter()
            .map(|record| LabelView {
                name: record.name,
                colour: record.colour.as_str().to_owned(),
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
        let name = tam_storage::labels::normalise(raw);
        if name.is_empty() {
            return Err(validation("a label needs a word in it"));
        }
        if name.chars().count() > 60 {
            return Err(validation("a label is at most sixty characters"));
        }
        if !names
            .iter()
            .any(|held: &String| held.eq_ignore_ascii_case(&name))
        {
            names.push(name);
        }
    }
    let records = LabelRepo::new(state.pool.clone())
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
            | StorageError::StaleLease
            | StorageError::DuplicateIdempotencyKey { .. }
            | StorageError::AttemptInFlight
            | StorageError::MappingAlreadyBound
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
    pub halted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raised_at: Option<Timestamp>,
}

const ALL_INVENTORIES: [InventoryId; 5] = [
    InventoryId::TesGb,
    InventoryId::TesUs,
    InventoryId::TesNz,
    InventoryId::Etsy,
    InventoryId::Tpt,
];

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
    /// The value pre-selected as a suggestion, where one exists. Never an
    /// answer: an election resolves only on explicit confirmation.
    pub suggested: Option<PathView>,
    /// What this listing loses on this target whatever the seller picks, so a
    /// Tes-to-TPT licence drop is visible at the moment of decision rather
    /// than after publish.
    pub losses: Vec<LossView>,
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

pub(crate) async fn list_decisions(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<DecisionsView>, APIError> {
    let repo = ElectionRepo::new(state.pool.clone());
    let taxonomy = TaxonomyRepo::new(state.pool.clone());
    let mappings = MappingRepo::new(state.pool.clone());
    let open = repo
        .open_items(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let mut items = Vec::with_capacity(open.len());
    for item in open {
        let vocabulary = VocabularyId(item.inventory, item.axis);
        let candidates: Vec<PathView> = taxonomy
            .edges_into(vocabulary)
            .await
            .map_err(|error| storage_fault(&state, &error))?
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Exact)
            .map(|edge| path_view(&edge.to))
            .collect();
        // The registry decides the mode, and `Never` wins over any opt-in.
        // The tenant opt-in is not modelled yet, so every axis reads as
        // seller-decides today and the licence axis will read that way even
        // once it is.
        let binding = registry(item.inventory).axis(item.axis);
        let mode = binding.map_or(Mode::SellerDecides, |binding| {
            resolution_for(binding, false)
        });
        // The losses of the mapping that raised the question. An election
        // authored on the create form names no mapping and so names no losses
        // yet, which is honest: nothing has been projected.
        let losses = match item.raised_by {
            Some(mapping) => mappings
                .losses(context.org, mapping)
                .await
                .map_err(|error| storage_fault(&state, &error))?
                .into_iter()
                .map(|loss| LossView {
                    kind: loss.kind.as_str().to_owned(),
                    axis: loss.axis,
                    detail: loss.detail,
                    recorded_at: loss.recorded_at,
                })
                .collect(),
            None => Vec::new(),
        };
        items.push(DecisionView {
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
            suggested: None,
            candidates,
            losses,
        });
    }
    Ok(Json(DecisionsView { items }))
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
