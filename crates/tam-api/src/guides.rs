//! The help guides: one document set an operator writes and every seller
//! reads.
//!
//! Two surfaces over one table. The operator's routes take
//! [`OperatorContext`], so the marking is checked before a handler runs; the
//! seller's take [`OrgContext`], because `/guides` sits behind the session
//! gate like every other console page and a guide is not public. Neither
//! surface uses the backoffice pool: `guide` is global (migration 0073) and
//! the application pool owns it, so there is no tenant fence here to cross
//! and no second connection to open.
//!
//! The seller's read filters on status in this module rather than in the
//! query, so a draft and a slug nobody has used answer identically: a 404
//! that distinguished them would publish the titles of guides that are not
//! published.
//!
//! # The body, and what becomes of raw HTML in it
//!
//! A guide is stored as the Markdown its author typed and rendered on the way
//! out, by [`render`]. That rendering escapes raw HTML into text rather than
//! passing it through: `Event::Html` and `Event::InlineHtml` are mapped to
//! `Event::Text` before the HTML is pushed, so `<script>` in a body arrives
//! at the console as `&lt;script&gt;` and a guide can never carry markup the
//! console did not write. The console renders the answered HTML directly into
//! the page, and that is safe exactly because of this: the sink is trusted
//! because the source is bounded here, once, rather than at each of the
//! places that display it.
//!
//! This is the opposite of what `tam-marketplace-tpt`'s own renderer does
//! with the same library, and deliberately: there the body is the seller's
//! copy travelling into an HTML field they own, and here it is one operator's
//! document rendered into every seller's browser.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, StatusCode};
use axum::Json;
use pulldown_cmark::{html::push_html, Event, Options, Parser};
use serde::{Deserialize, Serialize};
use tam_pipeline::store::LocalObjectStore;
use tam_storage::{
    ensure_platform_org, platform_org, BlobError, BlobRepo, GuideEdit, GuideHead, GuideRecord,
    GuideRepo, GuideStatus, GuideWrite, NewGuide,
};
use tam_types::{Timestamp, UserId};

use crate::catalogue::parse_hash;
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::resources::image_answer;
use crate::session::OperatorContext;
use crate::{AppState, OrgContext};

/// The extensions a guide body is rendered under.
///
/// Tables, because a guide that compares two marketplaces is a table.
/// Strikethrough, because it costs nothing and reads as itself when the
/// extension is off. Smart punctuation stays off for the reason
/// `tam-marketplace-tpt` keeps it off: it rewrites quotes and dashes into
/// other characters, which edits the copy rather than rendering it.
const MARKDOWN_EXTENSIONS: Options = Options::ENABLE_TABLES.union(Options::ENABLE_STRIKETHROUGH);

/// How long a guide's title may be, matching `guide_title_present` in
/// migration 0073 so a refusal is a sentence rather than a constraint
/// violation.
const TITLE_MAX_CHARS: usize = 120;

/// How long a guide's slug may be, matching `guide_slug_shape`.
const SLUG_MAX_CHARS: usize = 80;

/// How large a guide body may be, matching `guide_body_bounded`: 200 KiB of
/// Markdown, measured in bytes because bytes are what it costs.
const BODY_MAX_BYTES: usize = 204_800;

/// One guide as a listing renders it.
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideHeadView {
    pub slug: String,
    pub title: String,
    pub status: String,
    pub updated_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<UserId>,
}

impl GuideHeadView {
    fn of(head: GuideHead) -> Self {
        Self {
            slug: head.slug,
            title: head.title,
            status: head.status.as_str().to_owned(),
            updated_at: head.updated_at,
            updated_by: head.updated_by,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GuidesView {
    pub guides: Vec<GuideHeadView>,
}

/// One guide as its editor reads it: the Markdown to edit and the HTML that
/// Markdown renders to.
///
/// Both, rather than one and a client-side renderer. `body` is what the
/// editor writes back, and `html` is what the seller will see — answered by
/// the same function that will answer it, so the preview cannot disagree with
/// the published page.
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideView {
    pub slug: String,
    pub title: String,
    pub body: String,
    pub html: String,
    pub status: String,
    pub updated_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<UserId>,
}

impl GuideView {
    fn of(guide: GuideRecord) -> Self {
        let html = render(&guide.body);
        Self {
            slug: guide.slug,
            title: guide.title,
            body: guide.body,
            html,
            status: guide.status.as_str().to_owned(),
            updated_at: guide.updated_at,
            updated_by: guide.updated_by,
        }
    }
}

/// One published guide as a seller reads it. No `body` and no `status`: the
/// reader has no editor to fill and every guide it can see is published, so
/// both fields would be answers to questions this surface does not ask.
#[derive(Debug, Serialize, Deserialize)]
pub struct PublishedGuideView {
    pub slug: String,
    pub title: String,
    pub html: String,
    pub updated_at: Timestamp,
}

/// The handle a stored guide picture is addressed by.
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideImageView {
    pub handle: String,
}

/// What a guide write carries. The slug is absent: a create takes it in the
/// body, and an update addresses the guide by it in the path and never moves
/// it.
#[derive(Debug, Deserialize)]
pub struct GuideBody {
    pub title: String,
    pub body: String,
    pub status: String,
}

/// What a create carries: a write plus the address to write it at.
#[derive(Debug, Deserialize)]
pub struct NewGuideBody {
    pub slug: String,
    pub title: String,
    pub body: String,
    pub status: String,
}

/// A guide body rendered to HTML, with raw HTML escaped into text.
///
/// The escape is the whole point and it is three lines: CommonMark says raw
/// HTML in a Markdown document is HTML, `pulldown-cmark` faithfully hands it
/// over as [`Event::Html`] or [`Event::InlineHtml`], and pushing those
/// verbatim would put whatever an author typed into every reader's page.
/// Mapping them to [`Event::Text`] sends them through the same escaping every
/// other run of text gets, so the markup is shown rather than run.
///
/// A pure function of the bytes and [`MARKDOWN_EXTENSIONS`], so the editor's
/// preview and the published page are the same rendering rather than two.
#[must_use]
pub fn render(body: &str) -> String {
    let mut rendered = String::with_capacity(body.len());
    // Every event is named, and the catch-all a reader would expect here is
    // deliberately absent: this is the fence that decides what markup reaches
    // a `{@html}` sink, so a variant this parser grows later must stop the
    // build and be classified by hand rather than pass through unexamined
    // under a wildcard.
    let events = Parser::new_ext(body, MARKDOWN_EXTENSIONS).map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        other @ (Event::Start(_)
        | Event::End(_)
        | Event::Text(_)
        | Event::Code(_)
        | Event::InlineMath(_)
        | Event::DisplayMath(_)
        | Event::FootnoteReference(_)
        | Event::SoftBreak
        | Event::HardBreak
        | Event::Rule
        | Event::TaskListMarker(_)) => other,
    });
    push_html(&mut rendered, events);
    rendered
}

fn storage_fault(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    state.internal(&error.to_string())
}

fn missing(what: &str) -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new(what)
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
    )
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

/// The shape `guide_slug_shape` accepts, checked here so a malformed slug is
/// a sentence rather than a constraint violation surfacing as a fault.
fn check_slug(slug: &str) -> Result<(), APIError> {
    let shaped = !slug.is_empty()
        && slug.len() <= SLUG_MAX_CHARS
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--")
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if shaped {
        Ok(())
    } else {
        Err(validation(
            "a guide's slug is lowercase letters, digits and single hyphens, \
             up to eighty characters",
        ))
    }
}

/// The title, body and status a write carries, validated together.
fn check_write(title: &str, body: &str, status: &str) -> Result<GuideStatus, APIError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > TITLE_MAX_CHARS {
        return Err(validation(
            "a guide carries a title, in at most a hundred and twenty characters",
        ));
    }
    if body.len() > BODY_MAX_BYTES {
        return Err(validation(
            "a guide body is at most two hundred kibibytes of Markdown",
        ));
    }
    GuideStatus::parse(status)
        .ok_or_else(|| validation("a guide is either draft or published, and nothing else"))
}

// ---------------------------------------------------------------- operator

pub(crate) async fn list_guides(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<GuidesView>, APIError> {
    let guides = GuideRepo::new(state.pool.clone())
        .list()
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(GuidesView {
        guides: guides.into_iter().map(GuideHeadView::of).collect(),
    }))
}

pub(crate) async fn create_guide(
    State(state): State<AppState>,
    operator: OperatorContext,
    Json(body): Json<NewGuideBody>,
) -> Result<(StatusCode, Json<GuideView>), APIError> {
    check_slug(&body.slug)?;
    let status = check_write(&body.title, &body.body, &body.status)?;
    let repo = GuideRepo::new(state.pool.clone());
    let written = repo
        .create(&NewGuide {
            slug: &body.slug,
            edit: GuideEdit {
                title: body.title.trim(),
                body: &body.body,
                status,
                updated_by: operator.user,
                at: (state.wall)(),
            },
        })
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if written == GuideWrite::SlugTaken {
        return Err(APIError::new(
            StatusCode::CONFLICT,
            APIErrorEntry::new("a guide already lives at that slug").kind(APIErrorKind::Validation),
        ));
    }
    Ok((
        StatusCode::CREATED,
        Json(read_guide(&state, &body.slug).await?),
    ))
}

pub(crate) async fn guide_detail(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, slug)): Path<(String, String)>,
) -> Result<Json<GuideView>, APIError> {
    Ok(Json(read_guide(&state, &slug).await?))
}

pub(crate) async fn update_guide(
    State(state): State<AppState>,
    operator: OperatorContext,
    Path((_version, slug)): Path<(String, String)>,
    Json(body): Json<GuideBody>,
) -> Result<Json<GuideView>, APIError> {
    let status = check_write(&body.title, &body.body, &body.status)?;
    let written = GuideRepo::new(state.pool.clone())
        .update(
            &slug,
            &GuideEdit {
                title: body.title.trim(),
                body: &body.body,
                status,
                updated_by: operator.user,
                at: (state.wall)(),
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if !written {
        return Err(missing("no such guide"));
    }
    Ok(Json(read_guide(&state, &slug).await?))
}

pub(crate) async fn delete_guide(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, slug)): Path<(String, String)>,
) -> Result<StatusCode, APIError> {
    let deleted = GuideRepo::new(state.pool.clone())
        .delete(&slug)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(missing("no such guide"))
    }
}

/// The guide as the editor reads it back, re-read after every write so the
/// editor and the database cannot disagree about what was stored.
async fn read_guide(state: &AppState, slug: &str) -> Result<GuideView, APIError> {
    let guide = GuideRepo::new(state.pool.clone())
        .get(slug)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("no such guide"))?;
    Ok(GuideView::of(guide))
}

// ------------------------------------------------------------------ images

/// A picture for a guide, stored under the reserved platform organisation.
///
/// The bytes go through the same seal-and-store path a seller's upload takes
/// ([`BlobRepo::put`]), under the organisation whose slug is `guides` — an
/// organisation no tenant can claim, created here on the first upload. That
/// is what makes a guide picture readable by every seller: `blob` is keyed
/// `(org_id, hash)` and the reader's own organisation is not the one that
/// sealed these bytes, so [`guide_image`] pins the platform organisation
/// rather than the caller's.
///
/// No quota is charged, and the omission is the decision rather than an
/// oversight: the stored-bytes cap `POST /{version}/uploads` enforces is a
/// tenant's allowance against their own plan, and the platform organisation
/// holds no plan and sells nothing. Charging it would mean granting ourselves
/// an entitlement to publish help, and a full quota would then stop a guide
/// from being written rather than stopping a tenant from overrunning theirs.
/// What bounds this route instead is the body limit below and the operator
/// marking: only an operator reaches it at all.
pub(crate) async fn upload_guide_image(
    State(state): State<AppState>,
    _operator: OperatorContext,
    body: Bytes,
) -> Result<(StatusCode, Json<GuideImageView>), APIError> {
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
    if tam_pipeline::probe::probe_kind(&body) != Some(tam_types::FileKind::Image) {
        return Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new("a guide picture is a PNG, JPEG, GIF or WebP")
                .code(APIErrorCode::UploadRejected)
                .kind(APIErrorKind::Validation),
        ));
    }
    let now = (state.wall)();
    let org = ensure_platform_org(&state.pool, now)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let hash = BlobRepo::new(
        state.pool.clone(),
        LocalObjectStore::new(blobs.root.clone()),
        blobs.kek.clone(),
    )
    .put(org, &body, now)
    .await
    .map_err(|error| blob_fault(&state, &error))?;
    Ok((
        StatusCode::CREATED,
        Json(GuideImageView {
            handle: hex_of(hash),
        }),
    ))
}

/// The bytes of one guide picture.
///
/// [`OrgContext`] authenticates and nothing more: the organisation it names
/// is the reader's, and the organisation this read pins is the platform one,
/// because the picture belongs to the guide rather than to whoever is looking
/// at it. That asymmetry is the reason this route exists instead of the
/// seller's own `GET /{version}/uploads/{handle}`, which pins the reader's
/// organisation and therefore answers 404 for every guide picture.
///
/// A deployment where nobody has uploaded a guide picture has no platform
/// organisation yet, and every handle answers 404 — the same answer a handle
/// that names nothing gets. This route creates nothing: a read that
/// provisioned an organisation would be a write wearing a GET.
pub(crate) async fn guide_image(
    State(state): State<AppState>,
    _context: OrgContext,
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
    let Some(org) = platform_org(&state.pool)
        .await
        .map_err(|error| storage_fault(&state, &error))?
    else {
        return Err(missing("no such guide picture"));
    };
    let bytes = BlobRepo::new(
        state.pool.clone(),
        LocalObjectStore::new(blobs.root.clone()),
        blobs.kek.clone(),
    )
    .get(org, hash)
    .await
    .map_err(|error| match error {
        BlobError::Missing => missing("no such guide picture"),
        // Ours, not the reader's: the row says these bytes exist and we could
        // not produce them, which is a fault to be seen rather than a picture
        // to be reported absent.
        fault @ (BlobError::Storage(_) | BlobError::Store(_) | BlobError::Crypto(_)) => {
            blob_fault(&state, &fault)
        }
    })?;
    image_answer(bytes)
}

fn blob_fault(state: &AppState, error: &BlobError) -> APIError {
    state.internal(&error.to_string())
}

/// The handle's own spelling: the hash as lowercase hex, which is what every
/// other route in this API calls a handle.
fn hex_of(hash: tam_types::ContentHash) -> String {
    use core::fmt::Write as _;
    let mut hex = String::with_capacity(64);
    for byte in hash.0 {
        let _unused: core::fmt::Result = write!(hex, "{byte:02x}");
    }
    hex
}

/// How large a guide picture may be. The seller upload's own ceiling, because
/// a picture is a picture whoever posted it.
pub(crate) fn image_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(
        usize::try_from(tam_limits::http::UPLOAD_BODY_BYTES_MAX).unwrap_or(usize::MAX),
    )
}

// -------------------------------------------------------------------- reader

pub(crate) async fn published_guides(
    State(state): State<AppState>,
    _context: OrgContext,
) -> Result<Json<GuidesView>, APIError> {
    let guides = GuideRepo::new(state.pool.clone())
        .published()
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(GuidesView {
        guides: guides.into_iter().map(GuideHeadView::of).collect(),
    }))
}

pub(crate) async fn published_guide(
    State(state): State<AppState>,
    _context: OrgContext,
    Path((_version, slug)): Path<(String, String)>,
) -> Result<Json<PublishedGuideView>, APIError> {
    let guide = GuideRepo::new(state.pool.clone())
        .get(&slug)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .filter(|guide| guide.status == GuideStatus::Published)
        .ok_or_else(|| missing("no such guide"))?;
    Ok(Json(PublishedGuideView {
        html: render(&guide.body),
        slug: guide.slug,
        title: guide.title,
        updated_at: guide.updated_at,
    }))
}
