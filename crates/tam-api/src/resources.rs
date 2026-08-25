//! The catalogue, connections and reconciliation surfaces: what M1i's
//! client renders and the founder's drain workflow drives. Connection
//! revocation travels through the broker's unix socket — the only role that
//! can tombstone the vault — and a deployment without the socket answers
//! 503 rather than pretending to revoke.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_domain::{Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId, VocabularyPath};
use tam_storage::{ConnectionRepo, DrainStats, LedgerCursor, ProductRepo, TaxonomyRepo};
use tam_types::{
    CanonicalTermId, ConnectionId, InventoryId, Marketplace, OrgId, PriceIntent, ProductId,
    ScanOutcome, Timestamp, Uuid,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::jobs::{decode_cursor, encode_cursor, PageParams};
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

pub(crate) async fn list_products(
    State(state): State<AppState>,
    context: OrgContext,
    Query(params): Query<PageParams>,
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
        .list_page(context.org, cursor, limit)
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
    pub price: PriceIntent,
    pub files: Vec<FileView>,
    pub subjects: Vec<CanonicalTermId>,
    pub grades: GradesView,
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

const fn role_str(role: tam_types::FileRole) -> &'static str {
    match role {
        tam_types::FileRole::Payload => "payload",
        tam_types::FileRole::Preview => "preview",
        tam_types::FileRole::Cover => "cover",
    }
}

const fn kind_str(kind: tam_types::FileKind) -> &'static str {
    match kind {
        tam_types::FileKind::Pdf => "pdf",
        tam_types::FileKind::Pptx => "pptx",
        tam_types::FileKind::Docx => "docx",
        tam_types::FileKind::Zip => "zip",
        tam_types::FileKind::Image => "image",
    }
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
    Ok(Json(ProductView {
        id: aggregate.id,
        title: aggregate.title.0,
        body: aggregate.body.body,
        price: aggregate.price,
        files,
        subjects: aggregate.subjects,
        grades,
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
    pub state: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub(crate) async fn list_connections(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<ConnectionsView>, APIError> {
    let rows = ConnectionRepo::new(state.pool.clone())
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(ConnectionsView {
        connections: rows
            .into_iter()
            .map(|row| ConnectionView {
                id: row.id,
                marketplace: row.marketplace,
                state: row.state,
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
    },
    #[serde(other)]
    Unexpected,
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
        Ok(BrokerAnswer::Error { detail }) => Err(state.internal(&detail)),
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
}

pub(crate) async fn resolve_item(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, item)): Path<(String, String)>,
    Json(body): Json<ResolveBody>,
) -> Result<StatusCode, APIError> {
    if body.segments.is_empty() {
        return Err(validation("an edge needs at least one path segment"));
    }
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
        kind: EdgeKind::Exact,
        decided_by: Decider::Human {
            user: context.user,
            org: context.org,
        },
        decided_at: (state.wall)(),
    };
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
