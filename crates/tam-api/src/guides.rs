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
//! # Saving is not publishing
//!
//! A guide is a working copy and a snapshot of one (migration 0075). The
//! operator's routes read and write the working copy; the seller's routes read
//! the snapshot and nothing else, because the storage methods they call —
//! [`GuideRepo::published`] and [`GuideRepo::published_page`] — name snapshot
//! columns exclusively. That is why the reader's 404 for a draft is no longer
//! a filter in this module: there is no draft in the answer to filter out, so
//! no future edit to a handler here can leak one. A draft and a slug nobody
//! has used still answer identically, because a 404 that distinguished them
//! would publish the titles of guides that are not published.
//!
//! Every write an operator makes is conditional on the guide they were
//! looking at: the `id` and the `revision` they last read, both named and
//! both checked. A stale save, publish, unpublish or delete changes nothing
//! and is answered 409 with what is stored now in `detail.expected_id` and
//! `detail.expected_revision`, so the editor keeps the operator's text and
//! reconciles rather than discarding a paragraph or shipping one nobody read.
//!
//! The identifier is there because the revision alone is not enough to name a
//! guide. A guide that is deleted frees its slug, and a guide written at that
//! slug afterwards starts at revision 1 like the first one did — so an editor
//! still holding the deleted guide at revision 1 would, on the revision
//! alone, be allowed to overwrite a different guide's text with it. Naming
//! the id refuses that write, and answering the current id lets the console
//! say "the guide you were editing is gone" rather than "try again".
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
//!
//! Two more things happen at that same seam, and they happen there for the
//! same reason. Link and image destinations are checked as parsed URLs before
//! any HTML is emitted ([`link_allowed`], [`image_allowed`]), so a
//! `javascript:` destination cannot reach an `href` at all rather than being
//! caught by a sanitiser downstream. And an image is emitted as markup this
//! module writes, with `referrerpolicy="no-referrer"`, because a direct HTTPS
//! image is a request a reader's browser makes to somebody else's server and
//! the page they were reading is not that server's business.

use std::collections::VecDeque;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::Json;
use pulldown_cmark::{html::push_html, CowStr, Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use tam_storage::{
    ensure_platform_org, escape_like, platform_org, BlobError, BlobRepo, GuideDelete, GuideEdit,
    GuideHead, GuideOrder, GuidePublication, GuidePublishedHead, GuidePublishedPage, GuideRecord,
    GuideRepo, GuideRevisionWrite, GuideSearch, GuideTaxon, GuideTaxonKind, GuideTaxonWrite,
    GuideTaxonomy, GuideWrite, NewGuide, PUBLISHED_PAGE_ROWS,
};
use tam_types::{Timestamp, UserId, Uuid};

use crate::catalogue::parse_hash;
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::resources::image_answer;
use crate::session::OperatorContext;
use crate::{AppState, OrgContext};

/// The extensions a guide body is rendered under.
///
/// Tables, because a guide that compares two marketplaces is a table.
/// Strikethrough, because it costs nothing and reads as itself when the
/// extension is off. Footnotes, because a guide that explains a marketplace's
/// rule needs to cite it without breaking the sentence. Smart punctuation
/// stays off for the reason `tam-marketplace-tpt` keeps it off: it rewrites
/// quotes and dashes into other characters, which edits the copy rather than
/// rendering it.
const MARKDOWN_EXTENSIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_FOOTNOTES);

/// How long a guide's title may be, matching `guide_title_present` in
/// migration 0073 so a refusal is a sentence rather than a constraint
/// violation.
const TITLE_MAX_CHARS: usize = 120;

/// How long a guide's slug may be, matching `guide_slug_shape`.
const SLUG_MAX_CHARS: usize = 80;

/// How large a guide body may be, matching `guide_body_bounded`: 200 KiB of
/// Markdown, measured in bytes because bytes are what it costs.
const BODY_MAX_BYTES: usize = 204_800;

/// How long a topic's or tag's name may be, matching
/// `guide_taxon_name_bounded` in migration 0075.
const TAXON_NAME_MAX_CHARS: usize = 80;

/// How many tags one guide may carry.
///
/// The twenty a seller's own resource may carry (migration 0046), for the same
/// reason: past twenty the chips stop being a way to find anything, and an
/// unbounded array in a request body is an unbounded write behind it.
const TAGS_MAX: usize = 20;

/// How long a reader's search text may be.
///
/// A phrase, not a document: this is matched with `ILIKE` against every
/// published body, and a caller who sends a kilobyte is not searching.
const SEARCH_MAX_CHARS: usize = 120;

/// How many published guides one page of the reader's listing holds.
///
/// Twenty-five, which is what every other listing in this console pages at.
/// The storage layer holds the same number and its own ceiling; this is the
/// size the route asks for rather than a second opinion about the maximum.
const GUIDE_PAGE_ROWS: u32 = PUBLISHED_PAGE_ROWS;

/// The highest page ordinal this listing will read.
///
/// A bound on the `OFFSET` rather than on the corpus: an address asking for
/// page nine million is asking the database to count nine million rows it is
/// then going to throw away. Past this the answer is an empty page, which is
/// the truth about it.
const GUIDE_PAGE_MAX: u32 = 10_000;

/// Title A-Z: the order a reader looking for a title they half remember wants,
/// and the listing's default.
const GUIDE_SORT_TITLE: &str = "title";

/// Newest publication first.
const GUIDE_SORT_NEWEST: &str = "newest";

/// One topic or tag as any surface renders it.
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideTaxonView {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    /// Whether an operator has withdrawn this word from the pickers. Answered
    /// to readers too, and it has to be: a retired word a published guide is
    /// still filed under keeps appearing on that guide, so the console needs
    /// to be able to render it as withdrawn rather than as current.
    pub retired: bool,
}

impl GuideTaxonView {
    fn of(taxon: GuideTaxon) -> Self {
        Self {
            id: taxon.id,
            slug: taxon.slug,
            name: taxon.name,
            retired: taxon.retired,
        }
    }

    fn many(taxa: Vec<GuideTaxon>) -> Vec<Self> {
        taxa.into_iter().map(Self::of).collect()
    }

    fn maybe(taxon: Option<GuideTaxon>) -> Option<Self> {
        taxon.map(Self::of)
    }
}

/// Both vocabularies at once.
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideTaxonomyView {
    pub topics: Vec<GuideTaxonView>,
    pub tags: Vec<GuideTaxonView>,
}

impl GuideTaxonomyView {
    fn of(taxonomy: GuideTaxonomy) -> Self {
        Self {
            topics: GuideTaxonView::many(taxonomy.topics),
            tags: GuideTaxonView::many(taxonomy.tags),
        }
    }
}

/// One guide as a listing renders it.
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideHeadView {
    /// Which guide this row is. Carried so a write made from a listing names
    /// the guide the operator confirmed: a slug is an address, and the guide
    /// at it can be deleted and replaced between the listing and the write.
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub status: String,
    /// The aggregate revision this listing's row was read at, which is what a
    /// write against it has to name.
    pub revision: u32,
    pub topic: Option<GuideTaxonView>,
    pub tags: Vec<GuideTaxonView>,
    pub updated_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<UserId>,
}

impl GuideHeadView {
    /// The operator's row: the working copy's title and filing, because this
    /// is the listing an operator edits from.
    fn of(head: GuideHead) -> Self {
        Self {
            id: head.id,
            slug: head.slug,
            title: head.title,
            status: head.status.as_str().to_owned(),
            revision: head.revision,
            topic: GuideTaxonView::maybe(head.topic),
            tags: GuideTaxonView::many(head.tags),
            updated_at: head.updated_at,
            updated_by: head.updated_by,
        }
    }

    /// The reader's row, out of the snapshot and nothing else.
    ///
    /// `updated_at` is the publication, not the last autosave, so an operator
    /// typing in a draft cannot reorder a seller's listing. `revision` is the
    /// snapshot's source revision rather than the guide's current one, for the
    /// same reason: the number a reader is given is a fact about what they are
    /// reading, not a count of edits they cannot see. `updated_by` is absent —
    /// who was typing is not a reader's business.
    fn of_published(head: GuidePublishedHead) -> Self {
        Self {
            id: head.id,
            slug: head.slug,
            title: head.title,
            status: "published".to_owned(),
            revision: head.source_revision,
            topic: GuideTaxonView::maybe(head.topic),
            tags: GuideTaxonView::many(head.tags),
            updated_at: head.published_at,
            updated_by: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GuidesView {
    pub guides: Vec<GuideHeadView>,
    /// How many guides the question names, which is not how many rows this
    /// answer carries.
    ///
    /// On the reader's listing this is a count over the whole published corpus
    /// under the same filters, taken by its own statement: a page length
    /// reported as a total is how "25 guides match" comes to be printed over a
    /// corpus of two hundred. On the operator's listing the answer is the whole
    /// set, so the two numbers coincide there — and the field still means the
    /// same thing.
    pub total: usize,
    /// Which page this is, counting from one, echoed back so a console can
    /// tell a page past the end from the first page.
    pub page: u32,
    /// How many rows a full page of this listing holds.
    pub page_size: u32,
    /// Whether asking for the next page would answer any rows. Derived from
    /// the total rather than from the page being full, so the last page of an
    /// exact multiple is not followed by an empty one.
    pub has_next: bool,
    /// The order this answer is in, out of the two this listing sorts by: a
    /// request that named no order is answered in the default one and says so
    /// here, and a request naming a word this listing does not sort by is
    /// refused rather than answered in some other order.
    pub sort: String,
}

impl GuidesView {
    /// The whole set as one page: the operator's listing, which is unpaged.
    fn of(guides: Vec<GuideHeadView>) -> Self {
        let rows = u32::try_from(guides.len()).unwrap_or(u32::MAX);
        Self {
            total: guides.len(),
            page: 1,
            page_size: rows,
            has_next: false,
            sort: GUIDE_SORT_NEWEST.to_owned(),
            guides,
        }
    }

    /// One page of a larger set: the reader's listing.
    ///
    /// `has_next` is arithmetic over the total and the window rather than a
    /// look at whether the page came back full, because a full last page is
    /// indistinguishable from a full middle one.
    fn page(guides: Vec<GuideHeadView>, total: u64, page: u32, page_size: u32, sort: &str) -> Self {
        let seen = u64::from(page.saturating_sub(1))
            .saturating_mul(u64::from(page_size))
            .saturating_add(u64::try_from(guides.len()).unwrap_or(u64::MAX));
        Self {
            guides,
            total: usize::try_from(total).unwrap_or(usize::MAX),
            page,
            page_size,
            has_next: seen < total,
            sort: sort.to_owned(),
        }
    }
}

/// What publication took a copy of, as the editor reads it back.
#[derive(Debug, Serialize, Deserialize)]
pub struct GuidePublishedSnapshotView {
    pub title: String,
    pub body: String,
    pub html: String,
    pub topic: Option<GuideTaxonView>,
    pub tags: Vec<GuideTaxonView>,
    pub source_revision: u32,
    pub published_at: Timestamp,
}

impl GuidePublishedSnapshotView {
    fn of(published: GuidePublication) -> Self {
        let html = render(&published.body);
        Self {
            title: published.title,
            body: published.body,
            html,
            topic: GuideTaxonView::maybe(published.topic),
            tags: GuideTaxonView::many(published.tags),
            source_revision: published.source_revision,
            published_at: published.published_at,
        }
    }
}

/// One guide as its editor reads it: the Markdown to edit, the HTML that
/// Markdown renders to, and what sellers are reading meanwhile.
///
/// Both renderings, rather than one and a client-side renderer. `body` is what
/// the editor writes back, and `html` is what the seller will see — answered
/// by the same function that will answer it, so the preview cannot disagree
/// with the published page.
///
/// `published` is the whole reason an editor that lost an acknowledgement does
/// not have to guess: between `revision` and `published.source_revision` it
/// can see exactly which of its writes landed and whether the landing was a
/// save or a publish.
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideView {
    /// Which guide this is, as opposed to where it lives. A write names it
    /// beside the revision, and a changed `id` at a slug the editor knows
    /// means the guide it was editing was deleted and replaced rather than
    /// edited.
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub body: String,
    pub html: String,
    pub status: String,
    pub revision: u32,
    pub topic: Option<GuideTaxonView>,
    pub tags: Vec<GuideTaxonView>,
    pub published: Option<GuidePublishedSnapshotView>,
    pub updated_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<UserId>,
}

impl GuideView {
    fn of(guide: GuideRecord) -> Self {
        let html = render(&guide.body);
        Self {
            id: guide.id,
            slug: guide.slug,
            title: guide.title,
            body: guide.body,
            html,
            status: guide.status.as_str().to_owned(),
            revision: guide.revision,
            topic: GuideTaxonView::maybe(guide.topic),
            tags: GuideTaxonView::many(guide.tags),
            published: guide.published.map(GuidePublishedSnapshotView::of),
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
    pub topic: Option<GuideTaxonView>,
    pub tags: Vec<GuideTaxonView>,
    pub updated_at: Timestamp,
}

/// A body rendered without being stored: what the editor's preview shows.
#[derive(Debug, Serialize, Deserialize)]
pub struct GuidePreviewView {
    pub html: String,
}

/// The handle a stored guide picture is addressed by.
#[derive(Debug, Serialize, Deserialize)]
pub struct GuideImageView {
    pub handle: String,
}

/// What a save carries. The slug is absent: a create takes it in the body, and
/// a save addresses the guide by it in the path and never moves it.
///
/// No status. Publication is its own route, so there is no word a save can
/// carry that publishes what it saves.
#[derive(Debug, Deserialize)]
pub struct GuideBody {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub topic_id: Option<String>,
    #[serde(default)]
    pub tag_ids: Vec<String>,
    /// The guide this save is against, and the revision of it the operator
    /// read. Both are required: a save that named only a revision could land
    /// on a different guide that reached the same revision at the same slug.
    pub expected_id: String,
    pub expected_revision: u32,
}

/// What a create carries: a write plus the address to write it at. Always a
/// draft, so there is no `expected_revision` to name — nothing is being
/// overwritten.
#[derive(Debug, Deserialize)]
pub struct NewGuideBody {
    pub slug: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub topic_id: Option<String>,
    #[serde(default)]
    pub tag_ids: Vec<String>,
}

/// What publish, unpublish and delete carry: the guide the operator was
/// looking at when they asked, and the revision of it.
#[derive(Debug, Deserialize)]
pub struct GuideRevisionBody {
    pub expected_id: String,
    pub expected_revision: u32,
}

/// What a preview carries: unsaved Markdown, and nothing to write it to.
#[derive(Debug, Deserialize)]
pub struct GuidePreviewBody {
    pub body: String,
}

/// What a new topic or tag carries. The kind is the path's.
#[derive(Debug, Deserialize)]
pub struct NewGuideTaxonBody {
    pub slug: String,
    pub name: String,
}

/// What an edit to a topic or tag carries. The slug is not here: it is the
/// word's address in a shared link, so a rename moves the name and not the
/// address.
#[derive(Debug, Deserialize)]
pub struct GuideTaxonBody {
    pub name: String,
    pub retired: bool,
}

/// What a reader is asking the published listing for: the narrowing, the
/// order, and which page of it.
///
/// `tags` is comma-separated identifiers rather than a repeated parameter,
/// because the console mirrors these three into the URL and the query-cache
/// key and one string per filter is one thing to compare.
///
/// Every field is a string because every one of them is text a reader can
/// hand-edit into the address, and each one is refused or defaulted according
/// to whether it names something. A topic or tag that is not an identifier is
/// a refusal, and so is a `sort` outside the two words this listing sorts by:
/// both name a thing, and answering a different question than the one asked
/// is worse than saying no. `page` is not a name but a position, and a
/// position that cannot be read is the first one — which is also what an
/// absent `page` and an absent `sort` ask for.
#[derive(Debug, Default, Deserialize)]
pub struct GuideFilters {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub page: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
}

// ------------------------------------------------------------------ render

/// A guide body rendered to HTML, with raw HTML escaped into text, unsafe
/// destinations dropped, and footnote definitions moved after the prose in
/// reference order.
///
/// The escape is three lines and it is the load-bearing one: CommonMark says
/// raw HTML in a Markdown document is HTML, `pulldown-cmark` faithfully hands
/// it over as [`Event::Html`] or [`Event::InlineHtml`], and pushing those
/// verbatim would put whatever an author typed into every reader's page.
/// Mapping them to [`Event::Text`] sends them through the same escaping every
/// other run of text gets, so the markup is shown rather than run.
///
/// A pure function of the bytes and [`MARKDOWN_EXTENSIONS`], so the editor's
/// preview and the published page are the same rendering rather than two.
#[must_use]
pub fn render(body: &str) -> String {
    let mut rendered = String::with_capacity(body.len());
    push_html(&mut rendered, normalise(body).into_iter());
    rendered
}

/// The event stream a guide is rendered from: the parser's, with the three
/// rewrites this module is responsible for applied.
///
/// Collected rather than streamed, because two of the three need to see the
/// whole document: a footnote definition cannot be placed until every
/// reference before it has been read, and an image's alt text is the events
/// between its start and its end. The buffer is bounded by
/// [`BODY_MAX_BYTES`], which the routes enforce before a body is stored and
/// before a preview is rendered.
fn normalise(body: &str) -> Vec<Event<'_>> {
    let mut prose: Vec<Event<'_>> = Vec::new();
    // Definitions in the order they appear in the source, each holding its own
    // Start and End so the writer emits the whole block when we place it.
    let mut definitions: Vec<(CowStr<'_>, Vec<Event<'_>>)> = Vec::new();
    let mut open: Option<(CowStr<'_>, Vec<Event<'_>>)> = None;
    // The names referenced from the prose, first mention first: the order the
    // definitions will be emitted in, and therefore the order the writer
    // numbers them in.
    let mut referenced: VecDeque<CowStr<'_>> = VecDeque::new();
    // An image being read: its accepted destination, if it had one, and the
    // alt text its children spell.
    let mut image: Option<(Option<CowStr<'_>>, String)> = None;
    // A link whose destination was refused. Its children stay, so the words
    // survive as words; its End has to be dropped with its Start.
    let mut refused_links: usize = 0;

    for event in Parser::new_ext(body, MARKDOWN_EXTENSIONS) {
        if image.is_some() {
            match event {
                Event::Text(text) | Event::Code(text) => {
                    if let Some((_, alt)) = image.as_mut() {
                        alt.push_str(&text);
                    }
                }
                Event::SoftBreak | Event::HardBreak => {
                    if let Some((_, alt)) = image.as_mut() {
                        alt.push(' ');
                    }
                }
                Event::End(TagEnd::Image) => {
                    let (destination, alt) = image.take().unwrap_or((None, String::new()));
                    // A refused image degrades to its alt text: the author's
                    // words are theirs, and it is the destination that was not
                    // admissible.
                    let written = match destination {
                        Some(source) => Event::Html(CowStr::from(image_html(&source, &alt))),
                        None => Event::Text(CowStr::from(alt)),
                    };
                    push_event(&mut prose, &mut open, written);
                }
                // Emphasis and the like inside alt text: the markup is not
                // renderable in an attribute, and its text has already been
                // taken above. Named rather than caught by a wildcard, for the
                // main match's reason: an event this parser grows later is a
                // build to fix, not a silent omission from a reader's page.
                Event::Start(_)
                | Event::End(_)
                | Event::InlineMath(_)
                | Event::DisplayMath(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::FootnoteReference(_)
                | Event::Rule
                | Event::TaskListMarker(_) => {}
            }
            continue;
        }
        match event {
            // `link_type`, `title` and `id` are dropped with the parser's own
            // image emission: this module writes the element, and a title
            // attribute is one more author-controlled string in markup nobody
            // asked for.
            Event::Start(Tag::Image { dest_url, .. }) => {
                image = Some((image_allowed(&dest_url).then_some(dest_url), String::new()));
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                if link_allowed(&dest_url) {
                    push_event(
                        &mut prose,
                        &mut open,
                        Event::Start(Tag::Link {
                            link_type,
                            dest_url,
                            title,
                            id,
                        }),
                    );
                } else {
                    refused_links += 1;
                }
            }
            Event::End(TagEnd::Link) => {
                if refused_links > 0 {
                    refused_links -= 1;
                } else {
                    push_event(&mut prose, &mut open, Event::End(TagEnd::Link));
                }
            }
            Event::Start(Tag::FootnoteDefinition(name)) => {
                // A definition opening inside one cannot happen in this
                // grammar; if the parser ever admits it, the inner block is
                // placed as its own definition rather than nested, which is
                // what the writer would render anyway.
                if let Some(started) = open.take() {
                    definitions.push(started);
                }
                open = Some((
                    name.clone(),
                    vec![Event::Start(Tag::FootnoteDefinition(name))],
                ));
            }
            Event::End(TagEnd::FootnoteDefinition) => {
                if let Some((name, mut events)) = open.take() {
                    events.push(Event::End(TagEnd::FootnoteDefinition));
                    definitions.push((name, events));
                }
            }
            Event::FootnoteReference(name) => {
                if open.is_none() {
                    referenced.push_back(name.clone());
                }
                push_event(&mut prose, &mut open, Event::FootnoteReference(name));
            }
            Event::Html(raw) | Event::InlineHtml(raw) => {
                push_event(&mut prose, &mut open, Event::Text(raw));
            }
            // Every event is named, and the catch-all a reader would expect
            // here is deliberately absent: this is the fence that decides what
            // markup reaches a `{@html}` sink, so a variant this parser grows
            // later must stop the build and be classified by hand rather than
            // pass through unexamined under a wildcard.
            other @ (Event::Start(_)
            | Event::End(_)
            | Event::Text(_)
            | Event::Code(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::SoftBreak
            | Event::HardBreak
            | Event::Rule
            | Event::TaskListMarker(_)) => push_event(&mut prose, &mut open, other),
        }
    }
    if let Some(started) = open.take() {
        definitions.push(started);
    }

    // The definitions, in the order their first reference was read.
    //
    // This placement is what makes the numbering right rather than merely
    // tidy. The pinned writer numbers a footnote on first encounter of either
    // a reference or a definition, so a definition written above the paragraph
    // that cites it would take number 1 from the reference that should have
    // had it. With every definition after all of the prose, the references are
    // encountered first, in reading order, and the definitions follow in the
    // same order.
    //
    // A reference inside a definition joins the queue behind the rest, so a
    // footnote cited only by another footnote still lands after the one that
    // cites it. A definition nothing references is emitted after the
    // referenced ones, in source order: nobody asked for it, and dropping an
    // author's paragraph is not this function's decision.
    while !definitions.is_empty() {
        let next = loop {
            match referenced.pop_front() {
                Some(name) => {
                    if let Some(index) =
                        definitions.iter().position(|(defined, _)| *defined == name)
                    {
                        break Some(index);
                    }
                }
                None => break None,
            }
        };
        let (_, events) = definitions.remove(next.unwrap_or(0));
        for event in &events {
            if let Event::FootnoteReference(name) = event {
                referenced.push_back(name.clone());
            }
        }
        prose.extend(events);
    }
    prose
}

/// Where one event goes: into the definition being read, or into the prose.
fn push_event<'a>(
    prose: &mut Vec<Event<'a>>,
    open: &mut Option<(CowStr<'a>, Vec<Event<'a>>)>,
    event: Event<'a>,
) {
    match open.as_mut() {
        Some((_, events)) => events.push(event),
        None => prose.push(event),
    }
}

/// One image element, written here rather than by the parser's own emission.
///
/// Two attributes are the reason. `referrerpolicy="no-referrer"` keeps the
/// address of the guide a seller is reading out of a request to somebody
/// else's image host, which is the disclosure a direct HTTPS image otherwise
/// makes on every page load. `loading="lazy"` is the same courtesy to a long
/// guide's reader that every other picture in this console gets.
///
/// The source has already passed [`image_allowed`], which refuses control
/// characters and whitespace, and both values are escaped: a caption
/// containing a quotation mark closes nothing.
fn image_html(source: &str, alt: &str) -> String {
    format!(
        "<img src=\"{}\" alt=\"{}\" referrerpolicy=\"no-referrer\" loading=\"lazy\" />",
        escape_attribute(source),
        escape_attribute(alt),
    )
}

/// Text as an HTML attribute value holds it.
fn escape_attribute(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for character in raw.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// The scheme a destination names, lowercased, or `None` where it names none.
///
/// RFC 3986's own shape — a letter then letters, digits, `+`, `-` and `.`,
/// ended by a colon — which is what makes this a parse rather than a search
/// for a substring. A colon that arrives after a `/`, a `?`, a `#` or any
/// other character is part of a path and not a scheme, so `notes/9:30` is the
/// relative reference it looks like, and `%6Aavascript:x` names no scheme in
/// any browser either.
fn scheme_of(raw: &str) -> Option<String> {
    let mut characters = raw.chars();
    let first = characters.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    let mut scheme = String::with_capacity(raw.len());
    scheme.push(first.to_ascii_lowercase());
    for character in characters {
        match character {
            ':' => return Some(scheme),
            letter if letter.is_ascii_alphanumeric() || matches!(letter, '+' | '-' | '.') => {
                scheme.push(letter.to_ascii_lowercase());
            }
            _other => return None,
        }
    }
    None
}

/// Whether a destination is one this module will put in markup at all.
///
/// Control characters and whitespace are refused outright, which is what
/// answers the `java\tscript:` and `java\0script:` family: a browser strips
/// those before parsing the scheme, so a scheme check that ran on the string
/// as typed would be checking a different string from the one that navigates.
/// A protocol-relative `//host/path` is refused because it inherits the page's
/// scheme and points off-site, and neither of the things this module admits —
/// the platform's own uploads and an author's HTTPS link — needs it.
///
/// A backslash is refused for the same reason as the whitespace, and it is the
/// subtler case: WHATWG URL parsing treats a backslash in the authority
/// position as a forward slash, so `/\host/x` is parsed and fetched as
/// `//host/x`. Checking the leading character alone would call that
/// site-relative and admit an off-site request — a disclosure to somebody
/// else's server for an image, and a navigation that reads as internal for a
/// link. Every backslash goes rather than those two shapes, because a
/// destination has no use for one: separators are forward slashes on both
/// sides of what this admits, and a URL carrying a literal backslash spells
/// it `%5C`.
fn destination_shaped(raw: &str) -> bool {
    !raw.is_empty()
        && !raw.starts_with("//")
        && !raw.chars().any(|character| {
            character.is_control() || character.is_whitespace() || character == '\\'
        })
}

/// Whether a link destination may reach an `href`.
///
/// `https` because that is the web, `mailto` because a guide that says "write
/// to support" should link it, a leading `/` because the console is one origin
/// with the guides in it, a leading `#` because a long guide links its own
/// sections, and a relative reference because it resolves against the guide's
/// own URL and can name nothing else. Everything with a scheme this does not
/// name — `javascript`, `data`, `vbscript`, `file`, and whatever is next — is
/// refused, and refused here, where the answer is "no anchor", rather than
/// downstream where it would be "an anchor somebody has to sanitise".
fn link_allowed(raw: &str) -> bool {
    if !destination_shaped(raw) {
        return false;
    }
    if raw.starts_with('/') || raw.starts_with('#') {
        return true;
    }
    match scheme_of(raw) {
        None => true,
        Some(scheme) => scheme == "https" || scheme == "mailto",
    }
}

/// Whether an image destination may reach a `src`.
///
/// [`link_allowed`] minus `mailto` and minus the fragment, which name nothing
/// an image could be, and it is otherwise the same list: `https` for an
/// author's own hosting and a site-relative path for
/// `/v1/guides/images/{handle}`, which is where an uploaded picture lives.
/// `data:` is refused with the rest — an inline image is a body the bound
/// cannot see and a vector for markup, and no guide needs one.
///
/// Nothing here fetches: the destination travels to the reader's browser and
/// the server never resolves it, so an image URL is not a request this
/// deployment makes.
fn image_allowed(raw: &str) -> bool {
    if !destination_shaped(raw) || raw.starts_with('#') {
        return false;
    }
    if raw.starts_with('/') {
        return true;
    }
    match scheme_of(raw) {
        None => true,
        Some(scheme) => scheme == "https",
    }
}

// ------------------------------------------------------------------ refusals

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

/// The guide at this slug is not the guide the caller was writing to.
///
/// Either somebody else wrote it since the caller read it, or the guide the
/// caller had is gone and another one lives at its address. Both travel in
/// `detail` — `expected_id` and `expected_revision` are what a write would
/// have to name to land now — which is the whole point of answering 409
/// rather than 412 with an empty body: the editor keeps the operator's text
/// and decides what to do with it.
///
/// The two cases are one status and one shape deliberately. The console tells
/// them apart by comparing the answered `expected_id` against the one it
/// sent: the same id is the ordinary collision it reconciles against, and a
/// different id is a different guide, which it must never write into and must
/// never silently adopt as the target of the text on screen. Nothing was
/// written in either case, so there is nothing to undo.
fn stale(id: Uuid, revision: u32) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(
            "the guide stored at this address is not the one you were writing to; \
             your text is still here, and writing again will name the guide and \
             revision that are stored now",
        )
        .kind(APIErrorKind::Validation)
        .detail(serde_json::json!({
            "expected_id": id.to_hyphenated(),
            "expected_revision": revision,
        })),
    )
}

/// What a conditional write settled on, as an answer.
///
/// The guide travels out of the repository rather than being re-read here,
/// and that is a correctness property rather than a saving: a read taken
/// after the write's transaction committed can see a second operator's write
/// and would report their revision as this caller's own. An editor told that
/// advances its base revision past a write it never saw, then overwrites it
/// on the next autosave without ever being offered the conflict.
fn settled(outcome: GuideRevisionWrite) -> Result<GuideRecord, APIError> {
    match outcome {
        GuideRevisionWrite::Written(stored) => Ok(*stored),
        GuideRevisionWrite::Stale { id, revision } => Err(stale(id, revision)),
        GuideRevisionWrite::Missing => Err(missing("We can't find that guide.")),
    }
}

// ---------------------------------------------------------------- validation

/// The shape `guide_slug_shape` accepts, checked here so a malformed slug is
/// a sentence rather than a constraint violation surfacing as a fault. Shared
/// with the taxonomy, whose `guide_taxon_slug_shape` is the same shape for the
/// same reason: both end up in a URL.
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

/// The title and body a write carries, validated together.
fn check_write(title: &str, body: &str) -> Result<(), APIError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > TITLE_MAX_CHARS {
        return Err(validation(
            "a guide carries a title, in at most a hundred and twenty characters",
        ));
    }
    check_body(body)
}

/// The bound `guide_body_bounded` states, checked before a render as well as
/// before a store: a preview of a body nothing could save is work nobody
/// asked for.
fn check_body(body: &str) -> Result<(), APIError> {
    if body.len() > BODY_MAX_BYTES {
        return Err(validation(
            "a guide body is at most two hundred kibibytes of Markdown",
        ));
    }
    Ok(())
}

/// Which vocabulary a path segment names.
fn check_kind(kind: &str) -> Result<GuideTaxonKind, APIError> {
    match kind {
        "topics" => Ok(GuideTaxonKind::Topic),
        "tags" => Ok(GuideTaxonKind::Tag),
        _other => Err(missing("no such guide vocabulary")),
    }
}

/// One identifier as the wire spells it.
fn check_id(raw: &str, what: &str) -> Result<Uuid, APIError> {
    Uuid::parse_hyphenated(raw).ok_or_else(|| validation(&format!("{what} is an identifier")))
}

/// The topic and tags a write or a filter names, resolved against the
/// vocabulary they claim to be in.
///
/// An identifier that names nothing, or names a word of the other kind, is a
/// refusal rather than a silently dropped filter: a console whose picker sent
/// an identifier this server does not have is a console reading a vocabulary
/// that has moved, and answering it an unfiltered listing would show a reader
/// results they did not ask for. A retired word resolves — retirement stops a
/// word being offered, not a guide already filed under it being saved again.
async fn check_taxa(
    state: &AppState,
    kind: GuideTaxonKind,
    ids: &[Uuid],
    what: &str,
) -> Result<(), APIError> {
    if ids.is_empty() {
        return Ok(());
    }
    let found = GuideRepo::new(state.pool.clone())
        .taxa(kind, ids)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    if found.len() == ids.len() {
        Ok(())
    } else {
        Err(validation(&format!(
            "{what} names a guide {} this server does not have",
            kind.as_str(),
        )))
    }
}

/// The topic and tag identifiers one request body carries, parsed and
/// resolved.
async fn check_filing(
    state: &AppState,
    topic_id: Option<&str>,
    tag_ids: &[String],
) -> Result<(Option<Uuid>, Vec<Uuid>), APIError> {
    if tag_ids.len() > TAGS_MAX {
        return Err(validation("a guide carries at most twenty tags"));
    }
    let topic = topic_id
        .map(|raw| check_id(raw, "a guide's topic"))
        .transpose()?;
    let tags = tag_ids
        .iter()
        .map(|raw| check_id(raw, "a guide's tag"))
        .collect::<Result<Vec<Uuid>, APIError>>()?;
    check_taxa(
        state,
        GuideTaxonKind::Topic,
        topic.as_slice(),
        "a guide's topic",
    )
    .await?;
    check_taxa(state, GuideTaxonKind::Tag, &tags, "a guide's tags").await?;
    Ok((topic, tags))
}

/// The name a topic or tag carries.
fn check_taxon_name(name: &str) -> Result<&str, APIError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > TAXON_NAME_MAX_CHARS {
        return Err(validation(
            "a topic or tag carries a name, in at most eighty characters",
        ));
    }
    Ok(name)
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
    Ok(Json(GuidesView::of(
        guides.into_iter().map(GuideHeadView::of).collect(),
    )))
}

pub(crate) async fn create_guide(
    State(state): State<AppState>,
    operator: OperatorContext,
    Json(body): Json<NewGuideBody>,
) -> Result<(StatusCode, Json<GuideView>), APIError> {
    check_slug(&body.slug)?;
    check_write(&body.title, &body.body)?;
    let (topic, tags) = check_filing(&state, body.topic_id.as_deref(), &body.tag_ids).await?;
    let written = GuideRepo::new(state.pool.clone())
        .create(&NewGuide {
            slug: &body.slug,
            edit: GuideEdit {
                title: body.title.trim(),
                body: &body.body,
                topic,
                tags: &tags,
                updated_by: operator.user,
                at: (state.wall)(),
            },
        })
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let stored = match written {
        GuideWrite::Stored(stored) => *stored,
        GuideWrite::SlugTaken => {
            return Err(APIError::new(
                StatusCode::CONFLICT,
                APIErrorEntry::new("a guide already lives at that slug")
                    .kind(APIErrorKind::Validation),
            ))
        }
    };
    Ok((StatusCode::CREATED, Json(GuideView::of(stored))))
}

pub(crate) async fn guide_detail(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, slug)): Path<(String, String)>,
) -> Result<Json<GuideView>, APIError> {
    Ok(Json(read_guide(&state, &slug).await?))
}

/// Writes the working copy, and nothing a seller can see.
pub(crate) async fn save_guide(
    State(state): State<AppState>,
    operator: OperatorContext,
    Path((_version, slug)): Path<(String, String)>,
    Json(body): Json<GuideBody>,
) -> Result<Json<GuideView>, APIError> {
    check_write(&body.title, &body.body)?;
    let expected_id = check_id(&body.expected_id, "expected_id")?;
    let (topic, tags) = check_filing(&state, body.topic_id.as_deref(), &body.tag_ids).await?;
    let written = GuideRepo::new(state.pool.clone())
        .save(
            &slug,
            &GuideEdit {
                title: body.title.trim(),
                body: &body.body,
                topic,
                tags: &tags,
                updated_by: operator.user,
                at: (state.wall)(),
            },
            expected_id,
            body.expected_revision,
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(GuideView::of(settled(written)?)))
}

/// Copies the working copy into what sellers read.
pub(crate) async fn publish_guide(
    State(state): State<AppState>,
    operator: OperatorContext,
    Path((_version, slug)): Path<(String, String)>,
    Json(body): Json<GuideRevisionBody>,
) -> Result<Json<GuideView>, APIError> {
    let expected_id = check_id(&body.expected_id, "expected_id")?;
    let written = GuideRepo::new(state.pool.clone())
        .publish(
            &slug,
            expected_id,
            body.expected_revision,
            operator.user,
            (state.wall)(),
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(GuideView::of(settled(written)?)))
}

/// Withdraws what sellers read, leaving the working copy where it is.
pub(crate) async fn unpublish_guide(
    State(state): State<AppState>,
    operator: OperatorContext,
    Path((_version, slug)): Path<(String, String)>,
    Json(body): Json<GuideRevisionBody>,
) -> Result<Json<GuideView>, APIError> {
    let expected_id = check_id(&body.expected_id, "expected_id")?;
    let written = GuideRepo::new(state.pool.clone())
        .unpublish(
            &slug,
            expected_id,
            body.expected_revision,
            operator.user,
            (state.wall)(),
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(GuideView::of(settled(written)?)))
}

pub(crate) async fn delete_guide(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, slug)): Path<(String, String)>,
    Json(body): Json<GuideRevisionBody>,
) -> Result<StatusCode, APIError> {
    let expected_id = check_id(&body.expected_id, "expected_id")?;
    let deleted = GuideRepo::new(state.pool.clone())
        .delete(&slug, expected_id, body.expected_revision)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match deleted {
        GuideDelete::Deleted => Ok(StatusCode::NO_CONTENT),
        GuideDelete::Stale { id, revision } => Err(stale(id, revision)),
        GuideDelete::Missing => Err(missing("We can't find that guide.")),
    }
}

/// Renders unsaved Markdown, storing nothing.
///
/// The same [`render`] the published page goes through, which is the only
/// reason this route is allowed to exist: a second renderer would be a preview
/// that can disagree with the page, and the disagreement would be discovered
/// by a reader. No slug, no revision and no write — an operator previewing a
/// paragraph has not decided to keep it, and a preview that autosaved would
/// make the draft the preview rather than the other way round.
///
/// It awaits nothing — rendering is CPU over a bounded buffer, with no row to
/// read or write — and it is `async` anyway because `axum::Handler` is
/// implemented for functions returning a future and nothing else.
pub(crate) async fn preview_guide(
    _operator: OperatorContext,
    Json(body): Json<GuidePreviewBody>,
) -> Result<Json<GuidePreviewView>, APIError> {
    check_body(&body.body)?;
    Ok(Json(GuidePreviewView {
        html: render(&body.body),
    }))
}

pub(crate) async fn admin_guide_taxonomy(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<GuideTaxonomyView>, APIError> {
    let taxonomy = GuideRepo::new(state.pool.clone())
        .taxonomy()
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(GuideTaxonomyView::of(taxonomy)))
}

pub(crate) async fn create_guide_taxon(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, kind)): Path<(String, String)>,
    Json(body): Json<NewGuideTaxonBody>,
) -> Result<(StatusCode, Json<GuideTaxonView>), APIError> {
    let kind = check_kind(&kind)?;
    check_slug(&body.slug)?;
    let name = check_taxon_name(&body.name)?;
    let written = GuideRepo::new(state.pool.clone())
        .create_taxon(kind, &body.slug, name, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match written {
        GuideTaxonWrite::Stored(taxon) => {
            Ok((StatusCode::CREATED, Json(GuideTaxonView::of(taxon))))
        }
        GuideTaxonWrite::SlugTaken => Err(APIError::new(
            StatusCode::CONFLICT,
            APIErrorEntry::new("that vocabulary already holds a word at that slug")
                .kind(APIErrorKind::Validation),
        )),
    }
}

/// Renames a topic or tag, or retires and unretires it.
///
/// There is no delete, and that is the decision rather than an omission: a
/// word a published guide is filed under cannot be removed without either
/// taking that guide's filing with it or refusing the operator at a foreign
/// key. Retirement withdraws the word from the pickers and leaves every guide
/// that names it intact and legible.
pub(crate) async fn update_guide_taxon(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, kind, id)): Path<(String, String, String)>,
    Json(body): Json<GuideTaxonBody>,
) -> Result<Json<GuideTaxonView>, APIError> {
    let kind = check_kind(&kind)?;
    let id = check_id(&id, "a topic or tag")?;
    let name = check_taxon_name(&body.name)?;
    let written = GuideRepo::new(state.pool.clone())
        .update_taxon(kind, id, name, body.retired)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such topic or tag"))?;
    Ok(Json(GuideTaxonView::of(written)))
}

/// The guide as the editor reads it, whole: the working copy, what is
/// published of it, and the revision both are at.
///
/// The read an editor whose acknowledgement was lost reconciles against, and
/// deliberately not the answer to a write: a write answers with the record
/// its own transaction produced ([`settled`]), because a read taken
/// afterwards can be overtaken by another operator's write.
async fn read_guide(state: &AppState, slug: &str) -> Result<GuideView, APIError> {
    let guide = GuideRepo::new(state.pool.clone())
        .get(slug)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("We can't find that guide."))?;
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
    let hash = BlobRepo::new(state.pool.clone(), blobs.object_store(), blobs.kek.clone())
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
        return Err(missing("We can't find that picture."));
    };
    let bytes = BlobRepo::new(state.pool.clone(), blobs.object_store(), blobs.kek.clone())
        .get(org, hash)
        .await
        .map_err(|error| match error {
            BlobError::Missing => missing("We can't find that picture."),
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

/// One page of the published guides a seller's filters name.
///
/// Search AND topic AND any-of-tags, which is the one contract this listing
/// has: three conditions narrowing one set, so adding a tag never widens a
/// result. The text is matched literally — `escape_like` makes a per-cent sign
/// a per-cent sign rather than a wildcard — and case-insensitively, against
/// the published title, the published prose and the names of the words the
/// guide is published under. A phrase that exists only in somebody's unsaved
/// edit matches nothing, because none of those columns is the working copy.
///
/// The narrowing happens in the database and so does the window: the filters
/// are applied to the whole published corpus and one page is taken out of the
/// result, so a search finds a guide on page six that a console paging through
/// twenty-five rows at a time would never have loaded. `total` beside the rows
/// is a count over the same narrowing, which is what lets a reader be told
/// "26–50 of 143" rather than the length of what they were sent.
pub(crate) async fn published_guides(
    State(state): State<AppState>,
    _context: OrgContext,
    Query(filters): Query<GuideFilters>,
) -> Result<Json<GuidesView>, APIError> {
    let text = filters
        .q
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    if text.is_some_and(|text| text.chars().count() > SEARCH_MAX_CHARS) {
        return Err(validation("Keep your search to 120 characters or fewer."));
    }
    let pattern = text.map(escape_like);
    let topic = filters
        .topic
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| check_id(id, "a topic filter"))
        .transpose()?;
    let tags = filters
        .tags
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| check_id(id, "a tag filter"))
        .collect::<Result<Vec<Uuid>, APIError>>()?;
    if tags.len() > TAGS_MAX {
        return Err(validation("Pick 20 tags or fewer."));
    }
    check_taxa(
        &state,
        GuideTaxonKind::Topic,
        topic.as_slice(),
        "a topic filter",
    )
    .await?;
    check_taxa(&state, GuideTaxonKind::Tag, &tags, "a tag filter").await?;

    let (order, sort) = asked_order(filters.sort.as_deref())?;
    let page = asked_page(filters.page.as_deref());
    let search = GuideSearch {
        text: pattern.as_deref(),
        topic,
        tags: &tags,
        order,
        skip: page.saturating_sub(1).saturating_mul(GUIDE_PAGE_ROWS),
        take: GUIDE_PAGE_ROWS,
    };
    let repo = GuideRepo::new(state.pool.clone());
    let guides = repo
        .published(&search)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    // Counted separately rather than windowed out of the rows, because a page
    // past the end carries no rows at all and a total taken from them would
    // report an empty corpus to a reader who has simply walked too far.
    let total = repo
        .published_count(&search)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(GuidesView::page(
        guides
            .into_iter()
            .map(GuideHeadView::of_published)
            .collect(),
        total,
        page,
        GUIDE_PAGE_ROWS,
        sort,
    )))
}

/// The order a `sort` word names, and the word for the order that was applied.
///
/// A closed vocabulary of two words, and a third word is a refusal rather than
/// a quiet fall back to the default: it is the rule the rest of this API's
/// query parameters follow — a cursor this server did not issue and an
/// identifier no taxon holds are both refused — and for the same reason.
/// Answering a different order than the one asked for is an answer to a
/// different question, and the caller cannot tell it happened except by
/// reading a field back. Absent is not unknown: a request that names no order
/// is asking for the listing's own, which is the alphabet.
fn asked_order(sort: Option<&str>) -> Result<(GuideOrder, &'static str), APIError> {
    match sort.map(str::trim).filter(|sort| !sort.is_empty()) {
        None | Some(GUIDE_SORT_TITLE) => Ok((GuideOrder::Title, GUIDE_SORT_TITLE)),
        Some(GUIDE_SORT_NEWEST) => Ok((GuideOrder::Newest, GUIDE_SORT_NEWEST)),
        Some(_) => Err(validation("Sort by title or by newest.")),
    }
}

/// The page a `page` word names, counting from one.
///
/// Anything that is not a page ordinal — a word, nothing, a zero, a negative
/// number — is the first page, and an ordinal past [`GUIDE_PAGE_MAX`] is held
/// there: a page is a position in an answer rather than a name for anything,
/// so there is nothing for a reader to have got wrong and nothing to refuse.
/// A page beyond the last one answers no rows beside the real total, which is
/// what is true about it.
fn asked_page(page: Option<&str>) -> u32 {
    page.map(str::trim)
        .and_then(|page| page.parse::<u32>().ok())
        .filter(|page| *page >= 1)
        .unwrap_or(1)
        .min(GUIDE_PAGE_MAX)
}

/// The words published guides are filed under: the reader's whole filter
/// vocabulary.
///
/// A projection of what is published rather than of the table, so a topic an
/// operator is still deciding about is not a filter a seller can select and
/// not a word they can read.
pub(crate) async fn guide_taxonomy(
    State(state): State<AppState>,
    _context: OrgContext,
) -> Result<Json<GuideTaxonomyView>, APIError> {
    let taxonomy = GuideRepo::new(state.pool.clone())
        .published_taxonomy()
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(GuideTaxonomyView::of(taxonomy)))
}

pub(crate) async fn published_guide(
    State(state): State<AppState>,
    _context: OrgContext,
    Path((_version, slug)): Path<(String, String)>,
) -> Result<Json<PublishedGuideView>, APIError> {
    let guide: GuidePublishedPage = GuideRepo::new(state.pool.clone())
        .published_page(&slug)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("We can't find that guide."))?;
    Ok(Json(PublishedGuideView {
        html: render(&guide.body),
        slug: guide.slug,
        title: guide.title,
        topic: GuideTaxonView::maybe(guide.topic),
        tags: GuideTaxonView::many(guide.tags),
        updated_at: guide.published_at,
    }))
}
