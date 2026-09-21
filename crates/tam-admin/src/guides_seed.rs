//! The guide corpus as files: `guides seed --dir docs/guides` reads the
//! Markdown a repository holds and makes the database say what the files say.
//!
//! The files are the source and the database is a copy of them. A seed reads
//! every `*.md` in one directory, files each guide under the topic and tags
//! its front matter names, saves it and publishes it. Nothing here deletes: a
//! guide the directory no longer carries is left alone, because a corpus this
//! command has never been told about is not evidence that a guide is unwanted.
//!
//! # Idempotent by content, not by timestamp
//!
//! Every write is conditional on the content having changed. A guide whose
//! stored draft *and* published snapshot already hash to what the file hashes
//! to is skipped entirely, so running the seed twice writes once and the
//! second run does not bump a revision, re-publish a guide sellers are
//! reading, or take a new snapshot of prose that did not move. The hash covers
//! exactly what a seed can write -- title, topic, tags, body -- so a change an
//! operator made in the console to any of those is seen as a difference and
//! overwritten by the file, which is what "the files are the source" means.
//!
//! # What the front matter carries and why the body loses it
//!
//! A file opens with a `---` block naming `topic:` and one `tags:` line. Both
//! are slugs of the guide vocabulary, created on first use and reused
//! afterwards, so the taxonomy is a consequence of the corpus rather than a
//! second list to keep in step with it.
//!
//! Three things are stripped before the body is stored: the front matter, the
//! `# Title` heading (the title is a column, and leaving it in the body prints
//! it twice), and the `<!-- shot: ... -->` lines that say which screen each
//! picture comes from. The shot comments are authoring instructions, and the
//! renderer in `tam-api` escapes raw HTML into text rather than dropping it,
//! so a comment left in the body would be shown to the reader verbatim.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tam_storage::{
    GuideEdit, GuidePublication, GuideRecord, GuideRepo, GuideRevisionWrite, GuideTaxonKind,
    GuideTaxonWrite, GuideWrite, NewGuide,
};
use tam_types::{Timestamp, UserId, Uuid};

/// The longest title the API accepts (`crates/tam-api/src/guides.rs`). Checked
/// here so a bad file is refused by the command that reads it rather than by a
/// route nobody is calling.
const TITLE_MAX_CHARS: usize = 120;

/// The largest body the API accepts, for [`TITLE_MAX_CHARS`]'s reason.
const BODY_MAX_BYTES: usize = 204_800;

/// One guide as a file says it is.
pub(crate) struct GuideFile {
    pub(crate) slug: String,
    pub(crate) title: String,
    pub(crate) topic: String,
    pub(crate) tags: Vec<String>,
    pub(crate) body: String,
}

/// What one guide's seed did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Created,
    Updated,
    Unchanged,
}

impl Outcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
        }
    }
}

type Failure = Box<dyn std::error::Error>;

impl GuideFile {
    /// What this guide is, as one number over everything a seed can write.
    ///
    /// Tags are sorted first: the file's order and the database's are two
    /// different orders over one set, and a hash that saw them as different
    /// would re-publish every guide on every run.
    #[must_use]
    pub(crate) fn fingerprint(&self) -> u64 {
        let mut tags: Vec<&str> = self.tags.iter().map(String::as_str).collect();
        tags.sort_unstable();
        fingerprint(&self.title, Some(&self.topic), &tags, &self.body)
    }

    /// How the command prints one guide when it is only reading.
    #[must_use]
    pub(crate) fn summary(&self) -> String {
        format!(
            "{}\t{}\ttopic={}\ttags={}",
            self.slug,
            self.title,
            self.topic,
            self.tags.join(",")
        )
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// One guide's content as a number, over the four things a seed writes.
fn fingerprint(title: &str, topic: Option<&str>, tags: &[&str], body: &str) -> u64 {
    let mut hash = FNV_OFFSET;
    absorb(&mut hash, title);
    absorb(&mut hash, topic.unwrap_or(""));
    for tag in tags {
        absorb(&mut hash, tag);
    }
    absorb(&mut hash, body);
    hash
}

/// One field into the hash, followed by a separator byte no field can contain,
/// so `("ab", "c")` and `("a", "bc")` do not hash alike.
fn absorb(hash: &mut u64, text: &str) {
    for byte in text.as_bytes() {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
    *hash ^= 0xff;
    *hash = hash.wrapping_mul(FNV_PRIME);
}

/// What the stored working copy currently hashes to.
fn draft_fingerprint(record: &GuideRecord) -> u64 {
    let mut tags: Vec<&str> = record.tags.iter().map(|tag| tag.slug.as_str()).collect();
    tags.sort_unstable();
    fingerprint(
        &record.title,
        record.topic.as_ref().map(|topic| topic.slug.as_str()),
        &tags,
        &record.body,
    )
}

/// What the published snapshot currently hashes to.
fn published_fingerprint(published: &GuidePublication) -> u64 {
    let mut tags: Vec<&str> = published.tags.iter().map(|tag| tag.slug.as_str()).collect();
    tags.sort_unstable();
    fingerprint(
        &published.title,
        published.topic.as_ref().map(|topic| topic.slug.as_str()),
        &tags,
        &published.body,
    )
}

/// Every `*.md` in one directory, slug order, parsed and validated.
///
/// Slug order rather than directory order, so two runs on two machines print
/// the same list and a diff of two runs is a diff of the corpus.
pub(crate) fn read_directory(dir: &Path) -> Result<Vec<GuideFile>, Failure> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "md") {
            paths.push(path);
        }
    }
    paths.sort();
    if paths.is_empty() {
        return Err(format!("{} holds no .md file", dir.display()).into());
    }
    paths.iter().map(|path| read_file(path)).collect()
}

/// One file, refusing rather than guessing at anything it does not say.
pub(crate) fn read_file(path: &Path) -> Result<GuideFile, Failure> {
    let slug = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("{} has no usable file name", path.display()))?
        .to_owned();
    if !slug_shaped(&slug) {
        return Err(format!(
            "{} is not a slug: lowercase letters, digits and hyphens only",
            path.display()
        )
        .into());
    }
    let text = read_to_string(path)?;
    parse(&slug, &text).map_err(|reason| format!("{}: {reason}", path.display()).into())
}

#[expect(
    clippy::disallowed_methods,
    reason = "the corpus is repository text bounded by BODY_MAX_BYTES, read once per file by a one-shot command; the ban is about unbounded uploads"
)]
fn read_to_string(path: &Path) -> Result<String, Failure> {
    Ok(std::fs::read_to_string(path)?)
}

fn slug_shaped(slug: &str) -> bool {
    !slug.is_empty()
        && slug.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

/// The front matter, the heading and the prose, out of one file's text.
fn parse(slug: &str, text: &str) -> Result<GuideFile, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Err("the file must open with a --- front-matter block".to_owned());
    }
    let mut topic: Option<String> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        let line = line.trim();
        if line == "---" {
            closed = true;
            break;
        }
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| format!("front-matter line {line:?} names no key"))?;
        match key.trim() {
            "topic" => topic = Some(value.trim().to_owned()),
            "tags" => {
                tags = value
                    .split(',')
                    .map(str::trim)
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            other => return Err(format!("front matter names no key {other:?}")),
        }
    }
    if !closed {
        return Err("the front-matter block is never closed with ---".to_owned());
    }
    let topic = topic.ok_or("front matter names no topic")?;
    if !slug_shaped(&topic) {
        return Err(format!("topic {topic:?} is not a slug"));
    }
    for tag in &tags {
        if !slug_shaped(tag) {
            return Err(format!("tag {tag:?} is not a slug"));
        }
    }

    let mut title: Option<String> = None;
    let mut body = String::with_capacity(text.len());
    for line in lines {
        if title.is_none() {
            if let Some(heading) = line.strip_prefix("# ") {
                title = Some(heading.trim().to_owned());
                continue;
            }
        }
        let trimmed = line.trim();
        if trimmed.starts_with("<!--") && trimmed.ends_with("-->") {
            continue;
        }
        body.push_str(line);
        body.push('\n');
    }
    let title = title.ok_or("the file carries no # heading to take a title from")?;
    if title.is_empty() || title.chars().count() > TITLE_MAX_CHARS {
        return Err(format!("a title must be 1 to {TITLE_MAX_CHARS} characters"));
    }
    let body = body.trim().to_owned();
    if body.is_empty() {
        return Err("the file carries no prose under its heading".to_owned());
    }
    if body.len() > BODY_MAX_BYTES {
        return Err(format!("a body must be at most {BODY_MAX_BYTES} bytes"));
    }
    Ok(GuideFile {
        slug: slug.to_owned(),
        title,
        topic,
        tags,
        body,
    })
}

/// The vocabulary, read once and extended as the corpus asks for words.
///
/// One read up front rather than a lookup per guide: seventeen guides share
/// five topics, and the second guide under a topic must find the word the
/// first one created rather than race it.
struct Vocabulary {
    topics: BTreeMap<String, Uuid>,
    tags: BTreeMap<String, Uuid>,
}

impl Vocabulary {
    async fn load(repo: &GuideRepo) -> Result<Self, Failure> {
        let stored = repo.taxonomy().await?;
        Ok(Self {
            topics: stored
                .topics
                .into_iter()
                .map(|taxon| (taxon.slug, taxon.id))
                .collect(),
            tags: stored
                .tags
                .into_iter()
                .map(|taxon| (taxon.slug, taxon.id))
                .collect(),
        })
    }

    /// The word this slug names, created if the vocabulary has none.
    ///
    /// `SlugTaken` is not a fault: another operator, or another process
    /// running this same command, created the word between the load and the
    /// insert. Re-reading the vocabulary answers with theirs.
    async fn word(
        &mut self,
        repo: &GuideRepo,
        kind: GuideTaxonKind,
        slug: &str,
        at: Timestamp,
    ) -> Result<Uuid, Failure> {
        let known = match kind {
            GuideTaxonKind::Topic => self.topics.get(slug),
            GuideTaxonKind::Tag => self.tags.get(slug),
        };
        if let Some(id) = known {
            return Ok(*id);
        }
        let name = display_name(slug);
        let id = match repo.create_taxon(kind, slug, &name, at).await? {
            GuideTaxonWrite::Stored(taxon) => taxon.id,
            GuideTaxonWrite::SlugTaken => {
                let refreshed = Self::load(repo).await?;
                let found = match kind {
                    GuideTaxonKind::Topic => refreshed.topics.get(slug).copied(),
                    GuideTaxonKind::Tag => refreshed.tags.get(slug).copied(),
                };
                found.ok_or_else(|| {
                    format!("{slug:?} is taken by a word of the other kind; rename it")
                })?
            }
        };
        match kind {
            GuideTaxonKind::Topic => self.topics.insert(slug.to_owned(), id),
            GuideTaxonKind::Tag => self.tags.insert(slug.to_owned(), id),
        };
        Ok(id)
    }
}

/// A slug as a word a reader sees: `getting-started` reads "Getting started".
fn display_name(slug: &str) -> String {
    let spaced = slug.replace('-', " ");
    let mut characters = spaced.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => spaced,
    }
}

/// Every guide in the directory, written and published.
///
/// One guide at a time, and one failing guide stops the pass: a corpus half
/// written is a corpus an operator can fix and re-run, while a pass that
/// carried on past a refusal would bury which file was wrong.
pub(crate) async fn seed(
    repo: &GuideRepo,
    files: &[GuideFile],
    author: UserId,
    at: Timestamp,
) -> Result<Vec<(String, Outcome)>, Failure> {
    let mut vocabulary = Vocabulary::load(repo).await?;
    let mut outcomes = Vec::with_capacity(files.len());
    for file in files {
        let outcome = seed_one(repo, &mut vocabulary, file, author, at).await?;
        outcomes.push((file.slug.clone(), outcome));
    }
    Ok(outcomes)
}

async fn seed_one(
    repo: &GuideRepo,
    vocabulary: &mut Vocabulary,
    file: &GuideFile,
    author: UserId,
    at: Timestamp,
) -> Result<Outcome, Failure> {
    let topic = vocabulary
        .word(repo, GuideTaxonKind::Topic, &file.topic, at)
        .await?;
    let mut tags = Vec::with_capacity(file.tags.len());
    for tag in &file.tags {
        tags.push(vocabulary.word(repo, GuideTaxonKind::Tag, tag, at).await?);
    }
    let edit = GuideEdit {
        title: &file.title,
        body: &file.body,
        topic: Some(topic),
        tags: &tags,
        updated_by: author,
        at,
    };
    let wanted = file.fingerprint();

    let Some(stored) = repo.get(&file.slug).await? else {
        let created = match repo
            .create(&NewGuide {
                slug: &file.slug,
                edit,
            })
            .await?
        {
            GuideWrite::Stored(record) => record,
            GuideWrite::SlugTaken => {
                return Err(format!(
                    "{:?} was written by someone else mid-run; run the seed again",
                    file.slug
                )
                .into())
            }
        };
        publish(repo, &file.slug, (created.id, created.revision), author, at).await?;
        return Ok(Outcome::Created);
    };

    let draft_matches = draft_fingerprint(&stored) == wanted;
    let published_matches = stored
        .published
        .as_ref()
        .is_some_and(|published| published_fingerprint(published) == wanted);
    if draft_matches && published_matches {
        return Ok(Outcome::Unchanged);
    }

    let (id, revision) = if draft_matches {
        (stored.id, stored.revision)
    } else {
        let written = written(
            repo.save(&file.slug, &edit, stored.id, stored.revision)
                .await?,
            &file.slug,
        )?;
        (written.id, written.revision)
    };
    publish(repo, &file.slug, (id, revision), author, at).await?;
    Ok(Outcome::Updated)
}

/// One guide's working copy taken as the snapshot sellers read.
///
/// The guide is named as the pair the conditional write compares against,
/// because an identifier and a revision are one address between them and
/// passing them apart is how they come apart.
async fn publish(
    repo: &GuideRepo,
    slug: &str,
    target: (Uuid, u32),
    author: UserId,
    at: Timestamp,
) -> Result<(), Failure> {
    let (id, revision) = target;
    written(repo.publish(slug, id, revision, author, at).await?, slug)?;
    Ok(())
}

/// The record a conditional write produced, or the sentence saying why there
/// is none.
///
/// A stale or missing answer means an operator is editing the same guide in
/// the console right now. The seed refuses rather than retrying: overwriting
/// somebody mid-sentence is what the revision check exists to prevent.
fn written(outcome: GuideRevisionWrite, slug: &str) -> Result<Box<GuideRecord>, Failure> {
    match outcome {
        GuideRevisionWrite::Written(record) => Ok(record),
        GuideRevisionWrite::Stale { id, revision } => Err(format!(
            "{slug:?} changed under the seed (now {} at revision {revision}); run the seed again",
            uuid::Uuid::from_bytes(id.0)
        )
        .into()),
        GuideRevisionWrite::Missing => {
            Err(format!("{slug:?} was deleted under the seed; run the seed again").into())
        }
    }
}

/// How the command prints what it did.
#[must_use]
pub(crate) fn report(outcomes: &[(String, Outcome)]) -> String {
    let mut lines = String::new();
    let (mut created, mut updated, mut unchanged) = (0_usize, 0_usize, 0_usize);
    for (slug, outcome) in outcomes {
        match outcome {
            Outcome::Created => created += 1,
            Outcome::Updated => updated += 1,
            Outcome::Unchanged => unchanged += 1,
        }
        lines.push_str(slug);
        lines.push('\t');
        lines.push_str(outcome.as_str());
        lines.push('\n');
    }
    let summary = format!(
        "{count} guide(s): {created} created, {updated} updated, {unchanged} unchanged",
        count = outcomes.len()
    );
    lines.push_str(&summary);
    lines
}
