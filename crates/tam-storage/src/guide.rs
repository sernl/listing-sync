//! The help guides: one global document set, written by an operator and read
//! by every seller.
//!
//! Everything here runs on the application pool, which owns the tables by
//! owning the database (db/init/01-app-role.sql), so the write path needs no
//! grant and the read path needs no second connection. The backoffice role is
//! granted nothing on `guide` (migration 0073) or on the taxonomy tables
//! (migration 0075): a guide is not one tenant's data, so there is no fence
//! for a cross-tenant reader to cross.
//!
//! No method here takes an organisation, and none can: `guide` carries no
//! `org_id`. The one organisation this module does name is the reserved
//! platform one that owns guide pictures, and it is named by slug rather than
//! by a compiled-in identifier so a database that has one already keeps it.
//!
//! # Two copies of every guide
//!
//! A guide is a working copy and, once published, a snapshot of one. The
//! working copy is `title`/`body`/`topic_id` and the `draft` tag assignments;
//! the snapshot is the `published_*` columns and the `published` tag
//! assignments. Saving writes the first and never the second, so an operator
//! fixing a sentence does not edit what sellers are reading mid-sentence.
//!
//! The split is enforced by which columns each read names, not by a flag every
//! reader must remember to check. [`GuideRepo::published`],
//! [`GuideRepo::published_page`] and [`GuideRepo::published_taxonomy`] name
//! snapshot columns exclusively, and a guide with no snapshot is invisible to
//! all three — which is why migration 0075 dropped the old `status` column
//! rather than keeping it: a word saying "published" beside an absent or stale
//! snapshot is exactly how a draft leaks.
//!
//! # Identity, revisions, and why every write is conditional
//!
//! A guide's `id` is the guide, and its slug is only where the guide lives.
//! The two come apart the moment an operator deletes a guide and writes a new
//! one at the same address: the slug is free again and the new guide starts at
//! revision 1, so a revision alone cannot tell the second guide from the first
//! one somebody is still editing in another tab. It says "1" about both.
//!
//! `revision` counts writes to the whole aggregate — content and taxonomy
//! alike — and every write increments it. [`GuideRepo::save`],
//! [`GuideRepo::publish`], [`GuideRepo::unpublish`] and [`GuideRepo::delete`]
//! take the identifier *and* the revision the caller last saw, and change
//! nothing unless the stored row still carries both, answering
//! [`GuideRevisionWrite::Stale`] with the identifier and revision that are
//! stored now otherwise. Naming both is what makes a write against a guide
//! that has since been replaced a refusal rather than an overwrite of
//! somebody else's new guide, and it is checked in the write's own statement:
//! a pre-read that compared the id would be a race the `UPDATE` is not.
//!
//! Two operators in one guide therefore collide loudly, a publish cannot ship
//! a draft its publisher never read, and an editor whose acknowledgement was
//! lost can read the guide back and see from `id`, `revision` and
//! [`GuidePublication::source_revision`] exactly which of its writes landed —
//! and whether the guide it was writing is still the guide at that address.

use std::collections::HashMap;

use sqlx::{PgPool, Postgres, Transaction};
use tam_types::{OrgId, Timestamp, UserId, Uuid};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::StorageError;

/// Whether a guide is visible to sellers.
///
/// Two states and no third: a guide is being written or it is published.
/// Withdrawal is a return to `Draft`, which is why there is no `Archived` —
/// an unpublished guide reads exactly as one that was never published, and a
/// third state would be a second way of spelling the same visibility.
///
/// No longer a stored column and no longer parsed from one: since migration
/// 0075 this is derived from whether the guide carries a published snapshot,
/// so the two cannot disagree. It survives as the wire's vocabulary, which the
/// API passes through rather than translating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideStatus {
    Draft,
    Published,
}

impl GuideStatus {
    /// The word the API answers, which the console's union also spells.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
        }
    }

    /// The state a snapshot's presence means.
    const fn of_publication(published: bool) -> Self {
        if published {
            Self::Published
        } else {
            Self::Draft
        }
    }
}

/// Which of the two guide vocabularies a word belongs to.
///
/// One enum over one table's discriminator, because a topic and a tag differ
/// in how many a guide may carry rather than in what a word is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideTaxonKind {
    Topic,
    Tag,
}

impl GuideTaxonKind {
    /// The column's spelling, which `guide_taxon_kind` constrains.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Topic => "topic",
            Self::Tag => "tag",
        }
    }
}

/// One word a guide can be filed under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideTaxon {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    /// Whether an operator has withdrawn this word from the pickers.
    ///
    /// Carried rather than filtered out, because a retired word a published
    /// guide is still filed under has to stay legible to the reader looking at
    /// that guide. Retirement is the whole of "stop using this": there is no
    /// delete, so no published content can lose its filing.
    pub retired: bool,
}

/// Both vocabularies at once: what an operator files a guide with, and what a
/// reader filters by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideTaxonomy {
    pub topics: Vec<GuideTaxon>,
    pub tags: Vec<GuideTaxon>,
}

/// One guide's working copy, whole, body included: the operator's read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideRecord {
    /// The guide itself, for as long as it exists, and never reused: a delete
    /// is a hard delete and a guide written at the same slug afterwards is a
    /// different guide with a different identifier. This is what a
    /// conditional write names beside the revision, because the revision is
    /// only a position within one guide's history and a replacement's history
    /// starts at 1 again.
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    /// Markdown as the operator typed it. The rendering is the API's, done on
    /// the way out, so a guide is never stored as the HTML of the day it was
    /// saved.
    pub body: String,
    pub status: GuideStatus,
    pub revision: u32,
    pub topic: Option<GuideTaxon>,
    pub tags: Vec<GuideTaxon>,
    pub updated_by: Option<UserId>,
    /// When the working copy was last written — an autosave, usually. Never
    /// what a reader is shown: that is [`GuidePublication::published_at`].
    pub updated_at: Timestamp,
    /// What sellers are reading, or `None` for a guide nothing has published.
    pub published: Option<GuidePublication>,
}

/// The copy of a guide that publication took, and everything a reader sees of
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidePublication {
    pub title: String,
    pub body: String,
    pub topic: Option<GuideTaxon>,
    pub tags: Vec<GuideTaxon>,
    /// The revision of the working copy this was copied from: the exact draft
    /// the publisher had acknowledged.
    pub source_revision: u32,
    pub published_at: Timestamp,
}

/// One guide's working copy without its body: what the operator's listing
/// renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideHead {
    /// Which guide the row is, so a write made straight from a listing names
    /// the guide the operator confirmed rather than whatever holds the slug
    /// by the time the write arrives.
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub status: GuideStatus,
    pub revision: u32,
    pub topic: Option<GuideTaxon>,
    pub tags: Vec<GuideTaxon>,
    pub updated_by: Option<UserId>,
    pub updated_at: Timestamp,
}

/// One published guide without its body: what the seller's listing renders.
///
/// A separate type from [`GuideHead`] rather than the same one with some
/// fields blanked, because every field here comes from the snapshot and none
/// from the working copy. There is no `updated_by`: who last touched the
/// working copy is not a reader's business, and the snapshot does not record a
/// publisher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidePublishedHead {
    /// The guide this row is, carried for the same reason the operator's row
    /// carries it: a slug identifies an address and not a document.
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub topic: Option<GuideTaxon>,
    pub tags: Vec<GuideTaxon>,
    pub source_revision: u32,
    pub published_at: Timestamp,
}

/// One published guide whole: the page a seller reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidePublishedPage {
    pub slug: String,
    pub title: String,
    pub body: String,
    pub topic: Option<GuideTaxon>,
    pub tags: Vec<GuideTaxon>,
    pub published_at: Timestamp,
}

/// What one save says a guide's working copy now is: everything about it
/// except where it lives.
///
/// The slug is not here because it is the address rather than a field. A
/// create takes one beside this ([`NewGuide`]); a save addresses the guide
/// by the slug it already has and never moves it, since renaming would break
/// every link already pointing at the guide.
///
/// No status: publication is [`GuideRepo::publish`] and withdrawal is
/// [`GuideRepo::unpublish`], so there is no way to publish by writing a word
/// into a save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuideEdit<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub topic: Option<Uuid>,
    pub tags: &'a [Uuid],
    pub updated_by: UserId,
    pub at: Timestamp,
}

/// A guide as it is first written: an address and what goes at it.
///
/// Always a draft: a create writes a working copy and nothing else, so a guide
/// cannot arrive published without a publish having been asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewGuide<'a> {
    pub slug: &'a str,
    pub edit: GuideEdit<'a>,
}

/// The order a reader's listing is answered in.
///
/// Two orders and no third, because the corpus is help the platform writes:
/// what a reader wants is the alphabet when they are looking for a title they
/// half remember, and the newest first when they are looking for what
/// changed. Every order ends at the slug, which is unique, so a page boundary
/// falls in the same place twice and no row can be shown on two pages or on
/// none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuideOrder {
    /// Newest publication first. The order this listing has always answered
    /// in, and the one a reader asking "what is new" wants.
    #[default]
    Newest,
    /// Title A-Z, compared case-insensitively so `Etsy` and `etsy` sort
    /// together rather than in two alphabets.
    Title,
}

/// How many published guides one page holds where a caller names no size.
pub const PUBLISHED_PAGE_ROWS: u32 = 25;

/// The most rows one read of the published listing will answer, whatever is
/// asked for.
///
/// A ceiling rather than a suggestion: the listing is a `LIKE` scan over every
/// published body, and one request is not a licence to run the whole corpus
/// through it. A caller wanting more asks for the next page.
pub const PUBLISHED_PAGE_MAX: u32 = 100;

/// Which published guides a reader's filters name, in which order, and which
/// page of them.
///
/// Three conditions, all over the snapshot: the text, one topic, and any of a
/// set of tags. Absent text and an empty tag set are no condition at all
/// rather than a condition nothing satisfies.
///
/// The window is part of the question rather than something the caller trims
/// off the answer: the filters narrow the whole published corpus and the
/// window takes one page out of the narrowing, so a reader on page four is
/// reading rows the database chose and not rows a console scrolled past.
/// [`Default`] is the first page in the order this listing has always used,
/// which is why it is written out rather than derived — a derived zero `take`
/// would be a default that answers nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuideSearch<'a> {
    /// Literal text, already escaped by [`escape_like`], matched
    /// case-insensitively against the snapshot's title, prose and the names of
    /// the words it is filed under.
    pub text: Option<&'a str>,
    pub topic: Option<Uuid>,
    pub tags: &'a [Uuid],
    pub order: GuideOrder,
    /// Rows of the narrowing to step over before the page starts.
    pub skip: u32,
    /// Rows the page holds at most, clamped to [`PUBLISHED_PAGE_MAX`].
    pub take: u32,
}

impl Default for GuideSearch<'_> {
    fn default() -> Self {
        Self {
            text: None,
            topic: None,
            tags: &[],
            order: GuideOrder::Newest,
            skip: 0,
            take: PUBLISHED_PAGE_ROWS,
        }
    }
}

/// What a create settled on.
///
/// A slug another guide already holds is an ordinary answer the caller turns
/// into a sentence, not a fault, for the reason [`crate::org::OrgWrite`]
/// gives: checking before the write is a race and the unique index is the
/// arbiter.
///
/// `Stored` carries the guide as this write left it, read inside the same
/// transaction: see [`GuideRevisionWrite::Written`] for why an answer must
/// never be assembled from a read taken afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuideWrite {
    Stored(Box<GuideRecord>),
    SlugTaken,
}

/// What a conditional write settled on.
///
/// `Stale` and `Missing` are distinct because the answers differ: a stale
/// write is a collision the editor recovers from by reconciling against the
/// revision named here, and a missing guide is a 404. Neither is a fault, and
/// neither changed anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuideRevisionWrite {
    /// The guide exactly as this write left it, read in the same transaction
    /// that wrote it and before that transaction committed.
    ///
    /// The record is the outcome rather than a revision number the caller
    /// then re-reads, and that is the difference between a correct
    /// acknowledgement and an overtaken one. A read taken after the commit is
    /// a second question: another operator's write can land in between, and
    /// the answer then reports their revision as though it were this write's.
    /// An editor believing that advances its base revision past a write it
    /// never saw and overwrites it on the next autosave without ever being
    /// told there was a conflict.
    ///
    /// Boxed because the record is two copies of a guide and the other two
    /// variants are a word and a number.
    Written(Box<GuideRecord>),
    /// The guide stored at that slug now, which is not the guide the caller
    /// named: another identifier, another revision, or both. Nothing was
    /// written.
    ///
    /// Both are answered because the caller's recovery differs. A matching
    /// `id` with a later `revision` is the ordinary collision — reconcile and
    /// write again. A different `id` is a different guide: the one the caller
    /// was editing has been deleted, and no revision the caller could name
    /// would make its text belong here.
    Stale { id: Uuid, revision: u32 },
    /// No guide lives at that slug.
    Missing,
}

/// What a conditional delete settled on.
///
/// Separate from [`GuideRevisionWrite`] because there is no record to answer
/// with: the row the caller would read back is the row this removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideDelete {
    Deleted,
    Stale { id: Uuid, revision: u32 },
    Missing,
}

/// What a taxonomy create settled on. `SlugTaken` is the unique index's
/// answer, for [`GuideWrite::SlugTaken`]'s reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuideTaxonWrite {
    Stored(GuideTaxon),
    SlugTaken,
}

/// The slug of the reserved organisation that owns guide pictures.
///
/// Refused to every tenant by the validator in `crates/tam-api/src/org.rs`
/// since slugs existed, so nothing can be holding it when the first upload
/// asks for it.
pub const PLATFORM_ORG_SLUG: &str = "guides";

/// The name that organisation carries. Ours rather than a tenant's, and it is
/// the one an operator sees if they ever read the row.
const PLATFORM_ORG_NAME: &str = "Teachouse";

/// The `scope` value naming the working copy's tags.
const SCOPE_DRAFT: &str = "draft";

/// Text as a `LIKE` pattern matches it literally.
///
/// `%`, `_` and the escape character itself are what a reader types and not
/// what they mean by it: somebody searching for "50% off" is searching for a
/// per-cent sign, and passing it through would match every guide. Every query
/// built from this names `ESCAPE '\'` explicitly, so the escape is the one
/// this function writes rather than the server's default of the day.
#[must_use]
pub fn escape_like(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

pub struct GuideRepo {
    pool: PgPool,
}

impl GuideRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ------------------------------------------------------------ taxonomy

    /// Every word in both vocabularies, retired ones included: the operator's
    /// management screen and their pickers read the same list, because a
    /// retired word still has to be shown on the guides already filed under
    /// it.
    pub async fn taxonomy(&self) -> Result<GuideTaxonomy, StorageError> {
        let rows = sqlx::query!(
            "SELECT id, kind, slug, name, retired FROM guide_taxon \
             ORDER BY retired, lower(name), slug",
        )
        .fetch_all(&self.pool)
        .await?;
        split_taxonomy(rows.into_iter().map(|row| {
            (
                row.kind,
                GuideTaxon {
                    id: uuid_from_db(row.id),
                    slug: row.slug,
                    name: row.name,
                    retired: row.retired,
                },
            )
        }))
    }

    /// The words published guides are actually filed under.
    ///
    /// The reader's whole vocabulary, and it is a projection of the snapshot
    /// rather than of the table: a topic an operator has created but nothing
    /// published carries is a filing decision in progress, and listing it
    /// would offer sellers a filter that answers nothing and disclose editorial
    /// work that is not out yet.
    pub async fn published_taxonomy(&self) -> Result<GuideTaxonomy, StorageError> {
        let rows = sqlx::query!(
            "SELECT x.id, x.kind, x.slug, x.name, x.retired FROM guide_taxon x \
             WHERE EXISTS ( \
                     SELECT 1 FROM guide g \
                      WHERE g.published_at IS NOT NULL AND g.published_topic_id = x.id) \
                OR EXISTS ( \
                     SELECT 1 FROM guide_tag_assignment a \
                      JOIN guide g ON g.id = a.guide_id \
                      WHERE a.taxon_id = x.id AND a.scope = 'published' \
                        AND g.published_at IS NOT NULL) \
             ORDER BY x.retired, lower(x.name), x.slug",
        )
        .fetch_all(&self.pool)
        .await?;
        split_taxonomy(rows.into_iter().map(|row| {
            (
                row.kind,
                GuideTaxon {
                    id: uuid_from_db(row.id),
                    slug: row.slug,
                    name: row.name,
                    retired: row.retired,
                },
            )
        }))
    }

    /// The words of one kind these identifiers name, in the order asked for
    /// where they exist and absent where they do not.
    ///
    /// The caller compares what it gets back against what it asked for: fewer
    /// rows means an identifier that names nothing, or names a word of the
    /// other kind, and that is a sentence for the caller to write rather than
    /// a foreign-key violation surfacing as a fault. Filtering on `kind` here
    /// is what keeps a topic out of a tag list, which no CHECK constraint can
    /// do across tables.
    ///
    /// Retired words resolve. Retirement stops a word being offered, not a
    /// guide already filed under it being saved again, and the reader's filter
    /// for a retired word still has to answer the guides carrying it.
    pub async fn taxa(
        &self,
        kind: GuideTaxonKind,
        ids: &[Uuid],
    ) -> Result<Vec<GuideTaxon>, StorageError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let wanted: Vec<uuid::Uuid> = ids.iter().map(|id| uuid_to_db(*id)).collect();
        let rows = sqlx::query!(
            "SELECT id, slug, name, retired FROM guide_taxon \
             WHERE kind = $1 AND id = ANY($2)",
            kind.as_str(),
            &wanted[..],
        )
        .fetch_all(&self.pool)
        .await?;
        let found: HashMap<uuid::Uuid, GuideTaxon> = rows
            .into_iter()
            .map(|row| {
                (
                    row.id,
                    GuideTaxon {
                        id: uuid_from_db(row.id),
                        slug: row.slug,
                        name: row.name,
                        retired: row.retired,
                    },
                )
            })
            .collect();
        Ok(wanted
            .iter()
            .filter_map(|id| found.get(id).cloned())
            .collect())
    }

    /// Adds a word to one of the vocabularies.
    pub async fn create_taxon(
        &self,
        kind: GuideTaxonKind,
        slug: &str,
        name: &str,
        at: Timestamp,
    ) -> Result<GuideTaxonWrite, StorageError> {
        let done = sqlx::query!(
            "INSERT INTO guide_taxon (id, kind, slug, name, retired, created_at) \
             VALUES ($1, $2, $3, $4, false, $5) \
             RETURNING id, slug, name, retired",
            uuid_to_db(Uuid(*uuid::Uuid::new_v4().as_bytes())),
            kind.as_str(),
            slug,
            name,
            timestamp_to_db(at)?,
        )
        .fetch_one(&self.pool)
        .await;
        match done {
            Ok(row) => Ok(GuideTaxonWrite::Stored(GuideTaxon {
                id: uuid_from_db(row.id),
                slug: row.slug,
                name: row.name,
                retired: row.retired,
            })),
            Err(sqlx::Error::Database(database))
                if database.constraint() == Some("guide_taxon_one_per_slug") =>
            {
                Ok(GuideTaxonWrite::SlugTaken)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Renames a word, or retires and unretires it, answering `None` where
    /// that kind holds no such word.
    ///
    /// The slug does not move, for the reason a guide's does not: it is the
    /// word's address in a shared link.
    pub async fn update_taxon(
        &self,
        kind: GuideTaxonKind,
        id: Uuid,
        name: &str,
        retired: bool,
    ) -> Result<Option<GuideTaxon>, StorageError> {
        let row = sqlx::query!(
            "UPDATE guide_taxon SET name = $3, retired = $4 \
             WHERE kind = $1 AND id = $2 \
             RETURNING id, slug, name, retired",
            kind.as_str(),
            uuid_to_db(id),
            name,
            retired,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| GuideTaxon {
            id: uuid_from_db(row.id),
            slug: row.slug,
            name: row.name,
            retired: row.retired,
        }))
    }

    // -------------------------------------------------------------- reading

    /// Every guide's working copy, newest edit first: the operator's listing.
    ///
    /// The working title and filing, because this is the screen an operator
    /// edits from: what it shows is what a save would write. The two listings
    /// no longer share one statement with a predicate as a parameter, as they
    /// did before migration 0075, and they must not: they read disjoint sets
    /// of columns, so one statement serving both would be one statement that
    /// can answer a reader with a draft.
    pub async fn list(&self) -> Result<Vec<GuideHead>, StorageError> {
        let rows = sqlx::query!(
            "SELECT g.id, g.slug, g.title, g.revision, g.updated_by, g.updated_at, \
                    (g.published_at IS NOT NULL) AS \"published!\", \
                    t.id AS \"topic_id?\", t.slug AS \"topic_slug?\", \
                    t.name AS \"topic_name?\", t.retired AS \"topic_retired?\", \
                    tags.ids AS \"tag_ids?\", tags.slugs AS \"tag_slugs?\", \
                    tags.names AS \"tag_names?\", tags.retireds AS \"tag_retireds?\" \
             FROM guide g \
             LEFT JOIN guide_taxon t ON t.id = g.topic_id \
             LEFT JOIN LATERAL ( \
                 SELECT array_agg(x.id ORDER BY lower(x.name), x.slug) AS ids, \
                        array_agg(x.slug ORDER BY lower(x.name), x.slug) AS slugs, \
                        array_agg(x.name ORDER BY lower(x.name), x.slug) AS names, \
                        array_agg(x.retired ORDER BY lower(x.name), x.slug) AS retireds \
                 FROM guide_tag_assignment a \
                 JOIN guide_taxon x ON x.id = a.taxon_id \
                 WHERE a.guide_id = g.id AND a.scope = 'draft') tags ON true \
             ORDER BY g.updated_at DESC, g.slug",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(GuideHead {
                    id: uuid_from_db(row.id),
                    slug: row.slug,
                    title: row.title,
                    status: GuideStatus::of_publication(row.published),
                    revision: revision_from_db(row.revision)?,
                    topic: taxon_from_columns(
                        row.topic_id,
                        row.topic_slug,
                        row.topic_name,
                        row.topic_retired,
                    )?,
                    tags: taxa_from_columns(
                        row.tag_ids,
                        row.tag_slugs,
                        row.tag_names,
                        row.tag_retireds,
                    )?,
                    updated_by: row.updated_by.map(|id| UserId(uuid_from_db(id))),
                    updated_at: timestamp_from_db(row.updated_at),
                })
            })
            .collect()
    }

    /// One page of the published guides a reader's filters name, in the order
    /// the search asks for.
    ///
    /// Every column is a snapshot column, including the time and the filing,
    /// so an unpublished edit cannot change this listing's order, its text or
    /// its taxonomy — and a phrase that exists only in a working copy matches
    /// nothing here. The three conditions are text AND topic AND any-of-tags,
    /// which is one query rather than a search service: the corpus is the help
    /// the platform writes, so `ILIKE` over it is a scan of a few hundred
    /// rows and an index nobody can use is not worth the write cost.
    ///
    /// Always bounded, and the bound is the database's: the filters are
    /// applied before `LIMIT`, so page two of a search is the search's second
    /// page and not the second page of everything filtered afterwards. Every
    /// order ends at `g.slug`, which is unique per guide, so two rows can
    /// never tie and a page boundary cannot drop or repeat one. How many rows
    /// the whole narrowing holds is [`Self::published_count`]: a page length
    /// is a page length and is not a count of the corpus.
    ///
    /// The order is chosen by a bound flag rather than by two statements,
    /// because the alternative is two copies of a twenty-line predicate that
    /// have to be kept identical: a filter fixed in one and not the other is a
    /// listing that answers a different set depending on how it is sorted.
    /// The unused key is `NULL` for every row of the answer, which no order
    /// can be made out of, so it contributes nothing.
    pub async fn published(
        &self,
        search: &GuideSearch<'_>,
    ) -> Result<Vec<GuidePublishedHead>, StorageError> {
        let tags: Vec<uuid::Uuid> = search.tags.iter().map(|id| uuid_to_db(*id)).collect();
        let by_title = matches!(search.order, GuideOrder::Title);
        let take = i64::from(search.take.min(PUBLISHED_PAGE_MAX));
        let skip = i64::from(search.skip);
        let rows = sqlx::query!(
            "SELECT g.id, g.slug, g.published_title AS \"title!\", \
                    g.published_revision AS \"source_revision!\", \
                    g.published_at AS \"published_at!\", \
                    t.id AS \"topic_id?\", t.slug AS \"topic_slug?\", \
                    t.name AS \"topic_name?\", t.retired AS \"topic_retired?\", \
                    tags.ids AS \"tag_ids?\", tags.slugs AS \"tag_slugs?\", \
                    tags.names AS \"tag_names?\", tags.retireds AS \"tag_retireds?\" \
             FROM guide g \
             LEFT JOIN guide_taxon t ON t.id = g.published_topic_id \
             LEFT JOIN LATERAL ( \
                 SELECT array_agg(x.id ORDER BY lower(x.name), x.slug) AS ids, \
                        array_agg(x.slug ORDER BY lower(x.name), x.slug) AS slugs, \
                        array_agg(x.name ORDER BY lower(x.name), x.slug) AS names, \
                        array_agg(x.retired ORDER BY lower(x.name), x.slug) AS retireds \
                 FROM guide_tag_assignment a \
                 JOIN guide_taxon x ON x.id = a.taxon_id \
                 WHERE a.guide_id = g.id AND a.scope = 'published') tags ON true \
             WHERE g.published_at IS NOT NULL \
               AND ($1::text IS NULL \
                    OR g.published_title ILIKE '%' || $1 || '%' ESCAPE '\\' \
                    OR g.published_body ILIKE '%' || $1 || '%' ESCAPE '\\' \
                    OR t.name ILIKE '%' || $1 || '%' ESCAPE '\\' \
                    OR EXISTS ( \
                         SELECT 1 FROM guide_tag_assignment a \
                          JOIN guide_taxon x ON x.id = a.taxon_id \
                          WHERE a.guide_id = g.id AND a.scope = 'published' \
                            AND x.name ILIKE '%' || $1 || '%' ESCAPE '\\')) \
               AND ($2::uuid IS NULL OR g.published_topic_id = $2) \
               AND (cardinality($3::uuid[]) = 0 \
                    OR EXISTS ( \
                         SELECT 1 FROM guide_tag_assignment a \
                          WHERE a.guide_id = g.id AND a.scope = 'published' \
                            AND a.taxon_id = ANY($3))) \
             ORDER BY CASE WHEN $4::bool THEN lower(g.published_title) END ASC, \
                      CASE WHEN $4::bool THEN NULL::timestamptz \
                           ELSE g.published_at END DESC, \
                      g.slug ASC \
             LIMIT $5 OFFSET $6",
            search.text,
            search.topic.map(uuid_to_db),
            &tags[..],
            by_title,
            take,
            skip,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(GuidePublishedHead {
                    id: uuid_from_db(row.id),
                    slug: row.slug,
                    title: row.title,
                    topic: taxon_from_columns(
                        row.topic_id,
                        row.topic_slug,
                        row.topic_name,
                        row.topic_retired,
                    )?,
                    tags: taxa_from_columns(
                        row.tag_ids,
                        row.tag_slugs,
                        row.tag_names,
                        row.tag_retireds,
                    )?,
                    source_revision: revision_from_db(row.source_revision)?,
                    published_at: timestamp_from_db(row.published_at),
                })
            })
            .collect()
    }

    /// How many published guides the same filters name, whatever page is being
    /// read.
    ///
    /// The same three conditions as [`Self::published`] and deliberately no
    /// window: this is the number a reader is shown beside "page two of six",
    /// and it is a fact about the narrowing rather than about the rows that
    /// happened to be sent. The order and the window are the only things the
    /// two statements differ by — a count that filtered differently from the
    /// page would put a total beside rows it does not describe.
    pub async fn published_count(&self, search: &GuideSearch<'_>) -> Result<u64, StorageError> {
        let tags: Vec<uuid::Uuid> = search.tags.iter().map(|id| uuid_to_db(*id)).collect();
        let row = sqlx::query!(
            "SELECT count(*) AS \"total!\" \
             FROM guide g \
             LEFT JOIN guide_taxon t ON t.id = g.published_topic_id \
             WHERE g.published_at IS NOT NULL \
               AND ($1::text IS NULL \
                    OR g.published_title ILIKE '%' || $1 || '%' ESCAPE '\\' \
                    OR g.published_body ILIKE '%' || $1 || '%' ESCAPE '\\' \
                    OR t.name ILIKE '%' || $1 || '%' ESCAPE '\\' \
                    OR EXISTS ( \
                         SELECT 1 FROM guide_tag_assignment a \
                          JOIN guide_taxon x ON x.id = a.taxon_id \
                          WHERE a.guide_id = g.id AND a.scope = 'published' \
                            AND x.name ILIKE '%' || $1 || '%' ESCAPE '\\')) \
               AND ($2::uuid IS NULL OR g.published_topic_id = $2) \
               AND (cardinality($3::uuid[]) = 0 \
                    OR EXISTS ( \
                         SELECT 1 FROM guide_tag_assignment a \
                          WHERE a.guide_id = g.id AND a.scope = 'published' \
                            AND a.taxon_id = ANY($3)))",
            search.text,
            search.topic.map(uuid_to_db),
            &tags[..],
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(u64::try_from(row.total).unwrap_or(0))
    }

    /// One guide's working copy and whatever is published of it, whatever its
    /// state: the operator's read, and the read an editor that lost an
    /// acknowledgement reconciles against.
    ///
    /// The seller's route asks [`Self::published_page`] instead of filtering
    /// this, so a draft's prose never crosses the API boundary at all.
    pub async fn get(&self, slug: &str) -> Result<Option<GuideRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        let found = record(&mut tx, slug).await?;
        tx.commit().await?;
        Ok(found)
    }

    /// One published guide, or `None` where the slug names a draft or nothing
    /// at all.
    ///
    /// One question rather than a read and a filter, and that is the whole
    /// reason this method exists beside [`Self::get`]: the working copy's
    /// prose, title, filing and autosave time are not in the answer, so no
    /// caller can leak them by forgetting a check. A draft and a slug nobody
    /// has used answer identically, because a 404 that told them apart would
    /// publish the titles of guides that are not published.
    pub async fn published_page(
        &self,
        slug: &str,
    ) -> Result<Option<GuidePublishedPage>, StorageError> {
        let row = sqlx::query!(
            "SELECT g.slug, g.published_title AS \"title!\", \
                    g.published_body AS \"body!\", g.published_at AS \"published_at!\", \
                    t.id AS \"topic_id?\", t.slug AS \"topic_slug?\", \
                    t.name AS \"topic_name?\", t.retired AS \"topic_retired?\", \
                    tags.ids AS \"tag_ids?\", tags.slugs AS \"tag_slugs?\", \
                    tags.names AS \"tag_names?\", tags.retireds AS \"tag_retireds?\" \
             FROM guide g \
             LEFT JOIN guide_taxon t ON t.id = g.published_topic_id \
             LEFT JOIN LATERAL ( \
                 SELECT array_agg(x.id ORDER BY lower(x.name), x.slug) AS ids, \
                        array_agg(x.slug ORDER BY lower(x.name), x.slug) AS slugs, \
                        array_agg(x.name ORDER BY lower(x.name), x.slug) AS names, \
                        array_agg(x.retired ORDER BY lower(x.name), x.slug) AS retireds \
                 FROM guide_tag_assignment a \
                 JOIN guide_taxon x ON x.id = a.taxon_id \
                 WHERE a.guide_id = g.id AND a.scope = 'published') tags ON true \
             WHERE g.slug = $1 AND g.published_at IS NOT NULL",
            slug,
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(GuidePublishedPage {
            slug: row.slug,
            title: row.title,
            body: row.body,
            topic: taxon_from_columns(
                row.topic_id,
                row.topic_slug,
                row.topic_name,
                row.topic_retired,
            )?,
            tags: taxa_from_columns(row.tag_ids, row.tag_slugs, row.tag_names, row.tag_retireds)?,
            published_at: timestamp_from_db(row.published_at),
        }))
    }

    // -------------------------------------------------------------- writing

    /// Writes a guide that does not exist yet, as a draft at revision 1.
    ///
    /// One transaction, so a guide is never stored with half of its filing:
    /// the row and its draft tag assignments arrive together or not at all.
    /// The answer is the guide this write stored, read before that transaction
    /// committed, for [`GuideRevisionWrite::Written`]'s reason.
    pub async fn create(&self, new: &NewGuide<'_>) -> Result<GuideWrite, StorageError> {
        let at = timestamp_to_db(new.edit.at)?;
        let mut tx = self.pool.begin().await?;
        let stored = sqlx::query!(
            "INSERT INTO guide \
             (id, slug, title, body, topic_id, revision, updated_by, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, 1, $6, $7, $7) \
             RETURNING id",
            uuid_to_db(Uuid(*uuid::Uuid::new_v4().as_bytes())),
            new.slug,
            new.edit.title,
            new.edit.body,
            new.edit.topic.map(uuid_to_db),
            uuid_to_db(new.edit.updated_by.0),
            at,
        )
        .fetch_one(&mut *tx)
        .await;
        let guide = match stored {
            Ok(row) => row.id,
            Err(sqlx::Error::Database(database)) if database.constraint() == Some("guide_slug") => {
                return Ok(GuideWrite::SlugTaken);
            }
            Err(error) => return Err(error.into()),
        };
        attach_tags(&mut tx, guide, SCOPE_DRAFT, new.edit.tags).await?;
        let stored = record(&mut tx, new.slug)
            .await?
            .ok_or_else(|| StorageError::CorruptRow {
                reason: format!("guide {:?} is absent inside its own insert", new.slug),
            })?;
        tx.commit().await?;
        Ok(GuideWrite::Stored(Box::new(stored)))
    }

    /// Rewrites a guide's working copy, and nothing a reader can see.
    ///
    /// The `UPDATE` is the fence: it matches on the identifier and the
    /// revision the caller last read, so a second operator who saved in
    /// between finds nothing to write and is told what is stored instead of
    /// overwriting the first operator's paragraph — and a save aimed at a
    /// guide that has since been deleted and replaced at the same slug finds
    /// nothing either, rather than landing on the replacement. The published
    /// columns are not in the `SET` list at all, which is what makes "saving
    /// is not publishing" a property of the statement rather than of a
    /// caller's discipline.
    pub async fn save(
        &self,
        slug: &str,
        edit: &GuideEdit<'_>,
        expected_id: Uuid,
        expected_revision: u32,
    ) -> Result<GuideRevisionWrite, StorageError> {
        let at = timestamp_to_db(edit.at)?;
        let mut tx = self.pool.begin().await?;
        let written = sqlx::query!(
            "UPDATE guide \
                SET title = $4, body = $5, topic_id = $6, updated_by = $7, \
                    updated_at = $8, revision = revision + 1 \
              WHERE slug = $1 AND id = $2 AND revision = $3 \
             RETURNING id, revision",
            slug,
            uuid_to_db(expected_id),
            revision_to_db(expected_revision),
            edit.title,
            edit.body,
            edit.topic.map(uuid_to_db),
            uuid_to_db(edit.updated_by.0),
            at,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(written) = written else {
            let outcome = refusal(&mut tx, slug).await?;
            tx.commit().await?;
            return Ok(outcome);
        };
        sqlx::query!(
            "DELETE FROM guide_tag_assignment WHERE guide_id = $1 AND scope = 'draft'",
            written.id,
        )
        .execute(&mut *tx)
        .await?;
        attach_tags(&mut tx, written.id, SCOPE_DRAFT, edit.tags).await?;
        acknowledge(tx, slug).await
    }

    /// Copies the working copy into the snapshot sellers read.
    ///
    /// Every field and every assignment, in one transaction, from the exact
    /// guide and revision the publisher named: the title, the prose, the topic
    /// and the tags a reader sees all come from the same instant, and
    /// `published_revision` records which one. A publish that names a stale
    /// revision publishes nothing, because the draft it was agreeing to is not
    /// the draft that is there; a publish that names a guide since deleted
    /// publishes nothing either, and in particular does not publish whatever
    /// somebody has written at that slug since.
    ///
    /// `updated_at` moves too: publishing is an edit, and the operator's
    /// listing is ordered by it. It is not what the reader is shown —
    /// `published_at` is — so the autosave clock and the publication clock
    /// stay separate facts.
    #[expect(
        clippy::too_many_arguments,
        reason = "the slug and immutable id identify the guide, the revision fences the write, \
                  and the actor and instant are its audit facts; a parameter bag would only \
                  rename those five values"
    )]
    pub async fn publish(
        &self,
        slug: &str,
        expected_id: Uuid,
        expected_revision: u32,
        by: UserId,
        at: Timestamp,
    ) -> Result<GuideRevisionWrite, StorageError> {
        let at_db = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        let written = sqlx::query!(
            "UPDATE guide \
                SET published_title    = title, \
                    published_body     = body, \
                    published_topic_id = topic_id, \
                    published_revision = revision, \
                    published_at       = $5, \
                    updated_by         = $4, \
                    updated_at         = $5, \
                    revision           = revision + 1 \
              WHERE slug = $1 AND id = $2 AND revision = $3 \
             RETURNING id, revision",
            slug,
            uuid_to_db(expected_id),
            revision_to_db(expected_revision),
            uuid_to_db(by.0),
            at_db,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(written) = written else {
            let outcome = refusal(&mut tx, slug).await?;
            tx.commit().await?;
            return Ok(outcome);
        };
        sqlx::query!(
            "DELETE FROM guide_tag_assignment WHERE guide_id = $1 AND scope = 'published'",
            written.id,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "INSERT INTO guide_tag_assignment (guide_id, taxon_id, scope) \
             SELECT guide_id, taxon_id, 'published' FROM guide_tag_assignment \
              WHERE guide_id = $1 AND scope = 'draft'",
            written.id,
        )
        .execute(&mut *tx)
        .await?;
        acknowledge(tx, slug).await
    }

    /// Withdraws the snapshot, leaving the working copy untouched.
    ///
    /// Explicit, and the only way back: a save cannot unpublish, so nobody
    /// withdraws a guide by accident while editing it. What the operator keeps
    /// is everything they had — the prose and the filing are the working copy,
    /// which this does not touch — and what sellers lose is the page, which is
    /// what withdrawal means. Conditional on the identifier as well as the
    /// revision, so a withdrawal aimed at a deleted guide cannot take down the
    /// page of the guide that replaced it.
    #[expect(
        clippy::too_many_arguments,
        reason = "the slug and immutable id identify the guide, the revision fences the write, \
                  and the actor and instant are its audit facts; a parameter bag would only \
                  rename those five values"
    )]
    pub async fn unpublish(
        &self,
        slug: &str,
        expected_id: Uuid,
        expected_revision: u32,
        by: UserId,
        at: Timestamp,
    ) -> Result<GuideRevisionWrite, StorageError> {
        let at_db = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        let written = sqlx::query!(
            "UPDATE guide \
                SET published_title    = NULL, \
                    published_body     = NULL, \
                    published_topic_id = NULL, \
                    published_revision = NULL, \
                    published_at       = NULL, \
                    updated_by         = $4, \
                    updated_at         = $5, \
                    revision           = revision + 1 \
              WHERE slug = $1 AND id = $2 AND revision = $3 \
             RETURNING id, revision",
            slug,
            uuid_to_db(expected_id),
            revision_to_db(expected_revision),
            uuid_to_db(by.0),
            at_db,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(written) = written else {
            let outcome = refusal(&mut tx, slug).await?;
            tx.commit().await?;
            return Ok(outcome);
        };
        sqlx::query!(
            "DELETE FROM guide_tag_assignment WHERE guide_id = $1 AND scope = 'published'",
            written.id,
        )
        .execute(&mut *tx)
        .await?;
        acknowledge(tx, slug).await
    }

    /// Deletes a guide, if it is still the guide the caller read.
    ///
    /// Conditional like every other write here, because the destructive one is
    /// the last place to accept "whatever is there now": the operator agreed to
    /// delete a particular guide at a particular revision, and neither a
    /// paragraph somebody else added since nor a different guide written at
    /// that slug since is part of that agreement. The assignments go with the
    /// row, by the cascade migration 0075 declares.
    pub async fn delete(
        &self,
        slug: &str,
        expected_id: Uuid,
        expected_revision: u32,
    ) -> Result<GuideDelete, StorageError> {
        let mut tx = self.pool.begin().await?;
        let deleted = sqlx::query!(
            "DELETE FROM guide WHERE slug = $1 AND id = $2 AND revision = $3 \
             RETURNING revision",
            slug,
            uuid_to_db(expected_id),
            revision_to_db(expected_revision),
        )
        .fetch_optional(&mut *tx)
        .await?;
        let outcome = match deleted {
            Some(_) => GuideDelete::Deleted,
            None => match refusal(&mut tx, slug).await? {
                GuideRevisionWrite::Stale { id, revision } => GuideDelete::Stale { id, revision },
                // `refusal` only reads and only answers these three; a delete
                // that matched nothing is either a collision and Stale, or an
                // absence, and `Written` is unreachable because nothing here
                // wrote.
                GuideRevisionWrite::Written(_) | GuideRevisionWrite::Missing => {
                    GuideDelete::Missing
                }
            },
        };
        tx.commit().await?;
        Ok(outcome)
    }
}

/// Rows of both kinds, split into the two vocabularies they belong to.
///
/// One function over a `(kind, taxon)` pair rather than the same loop written
/// beside each query: the two reads differ by a `WHERE` clause and must not
/// differ in how they classify a word or in what they do with a kind the
/// CHECK constraint would not have accepted.
fn split_taxonomy(
    rows: impl IntoIterator<Item = (String, GuideTaxon)>,
) -> Result<GuideTaxonomy, StorageError> {
    let mut taxonomy = GuideTaxonomy {
        topics: Vec::new(),
        tags: Vec::new(),
    };
    for (kind, taxon) in rows {
        match kind.as_str() {
            "topic" => taxonomy.topics.push(taxon),
            "tag" => taxonomy.tags.push(taxon),
            other => {
                return Err(StorageError::CorruptRow {
                    reason: format!("unknown guide taxon kind {other:?}"),
                })
            }
        }
    }
    Ok(taxonomy)
}

/// The acknowledgement a successful conditional write answers with: the guide
/// as this transaction left it, read before the commit.
///
/// Taking the transaction by value is the point — the read and the commit are
/// one step, and there is no way to spell "commit, then read", which is the
/// sequence that hands an editor somebody else's revision.
async fn acknowledge(
    mut tx: Transaction<'_, Postgres>,
    slug: &str,
) -> Result<GuideRevisionWrite, StorageError> {
    let stored = record(&mut tx, slug)
        .await?
        .ok_or_else(|| StorageError::CorruptRow {
            reason: format!("guide {slug:?} is absent inside the write that changed it"),
        })?;
    tx.commit().await?;
    Ok(GuideRevisionWrite::Written(Box::new(stored)))
}

/// One guide whole, read inside the caller's transaction.
///
/// A free function over a transaction rather than a method on the pool,
/// because every conditional write answers with it *before committing*: the
/// record a caller gets back has to be the state that write produced, and a
/// read taken after the commit is a different question with a different
/// answer whenever a second operator wrote in between.
async fn record(
    tx: &mut Transaction<'_, Postgres>,
    slug: &str,
) -> Result<Option<GuideRecord>, StorageError> {
    let row = sqlx::query!(
        "SELECT g.id, g.slug, g.title, g.body, g.revision, g.updated_by, g.updated_at, \
                g.published_title, g.published_body, g.published_revision, \
                g.published_at, \
                t.id AS \"topic_id?\", t.slug AS \"topic_slug?\", \
                t.name AS \"topic_name?\", t.retired AS \"topic_retired?\", \
                p.id AS \"published_topic_id?\", p.slug AS \"published_topic_slug?\", \
                p.name AS \"published_topic_name?\", \
                p.retired AS \"published_topic_retired?\", \
                draft.ids AS \"tag_ids?\", draft.slugs AS \"tag_slugs?\", \
                draft.names AS \"tag_names?\", draft.retireds AS \"tag_retireds?\", \
                live.ids AS \"published_tag_ids?\", \
                live.slugs AS \"published_tag_slugs?\", \
                live.names AS \"published_tag_names?\", \
                live.retireds AS \"published_tag_retireds?\" \
         FROM guide g \
         LEFT JOIN guide_taxon t ON t.id = g.topic_id \
         LEFT JOIN guide_taxon p ON p.id = g.published_topic_id \
         LEFT JOIN LATERAL ( \
             SELECT array_agg(x.id ORDER BY lower(x.name), x.slug) AS ids, \
                    array_agg(x.slug ORDER BY lower(x.name), x.slug) AS slugs, \
                    array_agg(x.name ORDER BY lower(x.name), x.slug) AS names, \
                    array_agg(x.retired ORDER BY lower(x.name), x.slug) AS retireds \
             FROM guide_tag_assignment a \
             JOIN guide_taxon x ON x.id = a.taxon_id \
             WHERE a.guide_id = g.id AND a.scope = 'draft') draft ON true \
         LEFT JOIN LATERAL ( \
             SELECT array_agg(x.id ORDER BY lower(x.name), x.slug) AS ids, \
                    array_agg(x.slug ORDER BY lower(x.name), x.slug) AS slugs, \
                    array_agg(x.name ORDER BY lower(x.name), x.slug) AS names, \
                    array_agg(x.retired ORDER BY lower(x.name), x.slug) AS retireds \
             FROM guide_tag_assignment a \
             JOIN guide_taxon x ON x.id = a.taxon_id \
             WHERE a.guide_id = g.id AND a.scope = 'published') live ON true \
         WHERE g.slug = $1",
        slug,
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    // `guide_publication_whole` makes these four agree, so a disagreement
    // is corruption rather than a state to render: a title with no prose
    // would otherwise reach a reader as an empty page.
    let published = match (
        row.published_title,
        row.published_body,
        row.published_revision,
        row.published_at,
    ) {
        (None, None, None, None) => None,
        (Some(title), Some(body), Some(revision), Some(at)) => Some(GuidePublication {
            title,
            body,
            topic: taxon_from_columns(
                row.published_topic_id,
                row.published_topic_slug,
                row.published_topic_name,
                row.published_topic_retired,
            )?,
            tags: taxa_from_columns(
                row.published_tag_ids,
                row.published_tag_slugs,
                row.published_tag_names,
                row.published_tag_retireds,
            )?,
            source_revision: revision_from_db(revision)?,
            published_at: timestamp_from_db(at),
        }),
        (title, body, revision, at) => {
            return Err(StorageError::CorruptRow {
                reason: format!(
                    "half a guide publication ({}, {}, {revision:?}, {at:?})",
                    title.is_some(),
                    body.is_some(),
                ),
            })
        }
    };
    Ok(Some(GuideRecord {
        id: uuid_from_db(row.id),
        slug: row.slug,
        title: row.title,
        body: row.body,
        status: GuideStatus::of_publication(published.is_some()),
        revision: revision_from_db(row.revision)?,
        topic: taxon_from_columns(
            row.topic_id,
            row.topic_slug,
            row.topic_name,
            row.topic_retired,
        )?,
        tags: taxa_from_columns(row.tag_ids, row.tag_slugs, row.tag_names, row.tag_retireds)?,
        updated_by: row.updated_by.map(|id| UserId(uuid_from_db(id))),
        updated_at: timestamp_from_db(row.updated_at),
        published,
    }))
}

/// Which guide a conditional write found at that slug instead, or that it
/// found none.
///
/// Read inside the same transaction as the write it explains, so the
/// identifier and revision it reports are ones the caller can act on rather
/// than ones that were already gone when they were read.
async fn refusal(
    tx: &mut Transaction<'_, Postgres>,
    slug: &str,
) -> Result<GuideRevisionWrite, StorageError> {
    let current = sqlx::query!("SELECT id, revision FROM guide WHERE slug = $1", slug)
        .fetch_optional(&mut **tx)
        .await?;
    match current {
        Some(row) => Ok(GuideRevisionWrite::Stale {
            id: uuid_from_db(row.id),
            revision: revision_from_db(row.revision)?,
        }),
        None => Ok(GuideRevisionWrite::Missing),
    }
}

/// Files one guide under a set of tags in one scope.
///
/// `unnest` rather than a statement per tag, and `ON CONFLICT DO NOTHING` so a
/// request naming the same tag twice files it once instead of failing on the
/// primary key.
async fn attach_tags(
    tx: &mut Transaction<'_, Postgres>,
    guide: uuid::Uuid,
    scope: &str,
    tags: &[Uuid],
) -> Result<(), StorageError> {
    if tags.is_empty() {
        return Ok(());
    }
    let ids: Vec<uuid::Uuid> = tags.iter().map(|id| uuid_to_db(*id)).collect();
    sqlx::query!(
        "INSERT INTO guide_tag_assignment (guide_id, taxon_id, scope) \
         SELECT $1, taxon, $3 FROM unnest($2::uuid[]) AS taxon \
         ON CONFLICT DO NOTHING",
        guide,
        &ids[..],
        scope,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The revision a column holds. Negative or absent is corruption:
/// `guide_revision_positive` refuses it, and a decode that named it as a
/// number would carry the corruption into the wire.
fn revision_from_db(revision: i32) -> Result<u32, StorageError> {
    u32::try_from(revision).map_err(|_| StorageError::CorruptRow {
        reason: format!("guide revision {revision} is not a revision"),
    })
}

/// The revision a conditional write compares against.
///
/// Saturating rather than fallible: a client naming a revision no `integer`
/// column can hold has named a revision no guide carries, and the write then
/// matches nothing and is answered as stale — which is the truth about it —
/// rather than as a fault.
fn revision_to_db(revision: u32) -> i32 {
    i32::try_from(revision).unwrap_or(i32::MAX)
}

/// One taxon out of a `LEFT JOIN`'s four columns, absent where the join found
/// nothing.
fn taxon_from_columns(
    id: Option<uuid::Uuid>,
    slug: Option<String>,
    name: Option<String>,
    retired: Option<bool>,
) -> Result<Option<GuideTaxon>, StorageError> {
    match (id, slug, name, retired) {
        (None, None, None, None) => Ok(None),
        (Some(id), Some(slug), Some(name), Some(retired)) => Ok(Some(GuideTaxon {
            id: uuid_from_db(id),
            slug,
            name,
            retired,
        })),
        _ => Err(StorageError::CorruptRow {
            reason: "half a guide taxon in a joined row".to_owned(),
        }),
    }
}

/// A tag set out of four parallel `array_agg` columns, empty where the guide
/// carries none.
///
/// Aggregated in the same statement as the guide rather than fetched by a
/// second query, so one row is one consistent answer: a publish landing
/// between two reads could otherwise show a reader last week's prose under
/// this week's tags.
fn taxa_from_columns(
    ids: Option<Vec<uuid::Uuid>>,
    slugs: Option<Vec<String>>,
    names: Option<Vec<String>>,
    retired: Option<Vec<bool>>,
) -> Result<Vec<GuideTaxon>, StorageError> {
    let (Some(ids), Some(slugs), Some(names), Some(retired)) = (ids, slugs, names, retired) else {
        return Ok(Vec::new());
    };
    if ids.len() != slugs.len() || ids.len() != names.len() || ids.len() != retired.len() {
        return Err(StorageError::CorruptRow {
            reason: format!(
                "guide tag columns of {}, {}, {} and {} entries",
                ids.len(),
                slugs.len(),
                names.len(),
                retired.len(),
            ),
        });
    }
    Ok(ids
        .into_iter()
        .zip(slugs)
        .zip(names)
        .zip(retired)
        .map(|(((id, slug), name), retired)| GuideTaxon {
            id: uuid_from_db(id),
            slug,
            name,
            retired,
        })
        .collect())
}

/// The organisation guide pictures belong to, created if this database has
/// none yet.
///
/// Looked up by slug rather than by a compiled-in identifier, so a database
/// that already carries the row keeps it whatever its id is. The insert names
/// the slug the API reserves and `ON CONFLICT DO NOTHING` makes two uploads
/// racing produce one organisation rather than one error.
///
/// Deliberately not called by a read: a `GET` that created a tenant would be
/// a write dressed as a read. The reader asks [`platform_org`] instead and
/// answers "no such picture" where nobody has uploaded one.
pub async fn ensure_platform_org(pool: &PgPool, at: Timestamp) -> Result<OrgId, StorageError> {
    sqlx::query!(
        "INSERT INTO organisation (id, name, slug, slug_deferred, created_at) \
         VALUES ($1, $2, $3, false, $4) ON CONFLICT DO NOTHING",
        uuid_to_db(Uuid(*uuid::Uuid::new_v4().as_bytes())),
        PLATFORM_ORG_NAME,
        PLATFORM_ORG_SLUG,
        timestamp_to_db(at)?,
    )
    .execute(pool)
    .await?;
    let row = sqlx::query!(
        "SELECT id FROM organisation WHERE slug = $1",
        PLATFORM_ORG_SLUG,
    )
    .fetch_one(pool)
    .await?;
    Ok(OrgId(uuid_from_db(row.id)))
}

/// The organisation guide pictures belong to, or `None` where no upload has
/// created it yet.
pub async fn platform_org(pool: &PgPool) -> Result<Option<OrgId>, StorageError> {
    let row = sqlx::query!(
        "SELECT id FROM organisation WHERE slug = $1",
        PLATFORM_ORG_SLUG,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| OrgId(uuid_from_db(row.id))))
}
