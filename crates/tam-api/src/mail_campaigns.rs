//! The operators' mail to sellers: compose, choose who gets it, send, and
//! keep a log.
//!
//! # The body
//!
//! The console's editor writes HTML. It is reduced here, on every write and
//! every preview, to what a mail client renders alike: paragraphs, two heading
//! levels, bold, italic, lists, links to web or mail addresses, line breaks
//! and pictures, either uploaded for mail (`/v1/mail/images/{handle}`) or on
//! somebody's HTTPS host. Everything else is dropped, and its text kept. The
//! stored body is that reduction ([`sanitise_body`]) and it is idempotent, so a
//! stored body re-reduced is itself.
//!
//! # Variables
//!
//! Tokens in the text — `@name`, `@first_name`, `@email`, `@org`, `@plan`,
//! `@date`, `@link` and `@unsubscribe` — are replaced per recipient when the
//! mail is rendered ([`render`]). Replacement happens on the parsed text, and
//! every value is escaped as it is written, so a seller whose name is
//! `<script>` gets their name in the mail and nothing else. A paragraph that
//! holds only `@link` is drawn as the brand button; anywhere else it is a link.
//! A token must stand alone: `hi@org.test` is an address, not `@org`.
//!
//! # The wrapper
//!
//! The same brand wrapper the identity service's mails use
//! (`auth/src/template.ts`): tables and inline CSS only, the wordmark on white,
//! Indigo and Teal, Poppins for headings and Inter for text, and the footer
//! "© Teachouse · Auckland 1072, New Zealand · Powered by PLE Group". Every
//! mail adds one more footer line with the unsubscribe link, whatever the body
//! says, and the relay adds the one-click `List-Unsubscribe` headers.
//!
//! # Sending
//!
//! A campaign is written with one `mail_campaign_recipient` row per recipient
//! (migration 0097), which is that recipient's outbox entry; tam-server's
//! drainer claims due rows, resolves the address from the identity service,
//! renders through [`render`], and records the outcome on the row. No address
//! is stored: the domain database does not know any seller's address.

use std::collections::{HashMap, HashSet};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_limits::Plan;
use tam_storage::{
    ensure_platform_org, platform_org, AudienceMember, BackofficeRepo, BlobError, BlobRepo,
    CampaignCounts, CampaignRecord, MailCampaignRepo, NewCampaign, NewRecipient, OperatorRepo,
    RecipientRecord,
};
use tam_types::{OrgId, Timestamp, UserId, Uuid};

use crate::catalogue::parse_hash;
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::resources::image_answer;
use crate::rich_text::{decode_entities, escape_attribute, escape_text, parse, Element, Node};
use crate::session::OperatorContext;
use crate::{AppState, OrgContext};

/// A subject line's ceiling, which the table's CHECK enforces as well.
const SUBJECT_MAX_CHARS: usize = 200;
/// A body's ceiling: the guide's, and the table's CHECK.
const BODY_MAX_BYTES: usize = 204_800;
/// A button label's ceiling.
const LABEL_MAX_CHARS: usize = 60;
/// A link's ceiling.
const URL_MAX_CHARS: usize = 2_000;
/// What the button says when the operator does not say.
const DEFAULT_LINK_LABEL: &str = "Open Teachouse";
/// How long a display name may run in a mail.
const NAME_MAX_CHARS: usize = 80;
/// The public path an uploaded mail picture is served from.
pub const IMAGE_PATH: &str = "/v1/mail/images/";
/// The public path an unsubscribe link opens.
pub const UNSUBSCRIBE_PATH: &str = "/v1/mail/unsubscribe";

// ------------------------------------------------------------------- colours
//
// The brand kit's, written out because a mail client has no stylesheet to
// read them from; the same literals `auth/src/template.ts` names.
const GROUND: &str = "#F8FAF8";
const SURFACE: &str = "#FFFFFF";
const PRIMARY: &str = "#1E2A5A";
const ACCENT: &str = "#00B894";
const ON_FILL: &str = "#FFFFFF";
const TEXT: &str = "#2D3748";
const MUTED: &str = "#64748B";
const LINE: &str = "#E2E8F0";
const DISPLAY: &str = "Poppins,'Segoe UI',Helvetica,Arial,sans-serif";
const BODY: &str = "Inter,'Segoe UI',-apple-system,Helvetica,Arial,sans-serif";

// =============================================================== the wire

/// Which sellers a campaign goes to, by what they hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Segment {
    All,
    /// The free plan, sold as "Look".
    Free,
    /// Any paid plan.
    Paid,
    Starter,
    /// Sold as "Sync".
    Subscriber,
    Studio,
}

impl Segment {
    /// Whether an organisation on `plan` is in this segment.
    #[must_use]
    pub const fn admits(self, plan: Plan) -> bool {
        match self {
            Self::All => true,
            Self::Free => matches!(plan, Plan::Free),
            Self::Paid => !matches!(plan, Plan::Free),
            Self::Starter => matches!(plan, Plan::Starter),
            Self::Subscriber => matches!(plan, Plan::Subscriber),
            Self::Studio => matches!(plan, Plan::Studio),
        }
    }
}

/// The audience filter as the operator chose it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailAudience {
    pub segment: Segment,
    #[serde(default = "yes")]
    pub exclude_operators: bool,
    /// Enforced at send time: the domain database holds no addresses, and
    /// the identity service's answer carries whether it vouches for one.
    #[serde(default = "yes")]
    pub verified_only: bool,
}

const fn yes() -> bool {
    true
}

/// What the operator wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailDraft {
    pub subject: String,
    pub body_html: String,
    #[serde(default)]
    pub link_url: Option<String>,
    #[serde(default)]
    pub link_label: Option<String>,
}

/// How many sellers a filter reaches, and who it leaves out and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailAudienceCount {
    pub recipients: u64,
    /// Every seller the segment matches, before the exclusions below.
    pub sellers: u64,
    pub operators_excluded: u64,
    pub opted_out: u64,
    /// Sellers with no sign-in identity, who have no address to mail.
    pub no_sign_in: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailCountsView {
    pub total: i64,
    pub queued: i64,
    pub sent: i64,
    pub failed: i64,
    pub skipped: i64,
}

impl MailCountsView {
    const fn of(counts: CampaignCounts) -> Self {
        Self {
            total: counts.total,
            queued: counts.queued,
            sent: counts.sent,
            failed: counts.failed,
            skipped: counts.skipped,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailCampaignSummary {
    pub id: Uuid,
    pub subject: String,
    pub audience: MailAudience,
    pub counts: MailCountsView,
    pub created_at: Timestamp,
    pub created_by: UserId,
    pub created_by_label: String,
    pub test: bool,
    pub deleted_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailRecipientView {
    pub user: UserId,
    pub auth_subject: Option<Uuid>,
    pub org_name: String,
    pub plan: String,
    pub status: String,
    pub attempts: i32,
    pub provider_id: Option<String>,
    pub error: Option<String>,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailCampaignDetail {
    #[serde(flatten)]
    pub summary: MailCampaignSummary,
    /// `None` once deleted.
    pub draft: Option<MailDraft>,
    pub recipients: Vec<MailRecipientView>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MailCampaignsView {
    pub campaigns: Vec<MailCampaignSummary>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MailPreviewView {
    pub subject: String,
    pub html: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MailImageView {
    pub handle: String,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MailQueuedView {
    pub id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct TestMailBody {
    pub draft: MailDraft,
    #[serde(default)]
    pub created_by_label: String,
}

#[derive(Debug, Deserialize)]
pub struct NewCampaignBody {
    pub draft: MailDraft,
    pub audience: MailAudience,
    #[serde(default)]
    pub created_by_label: String,
}

// ===================================================== the body, reduced

/// Inline content of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Em(Vec<Inline>),
    Link { href: String, children: Vec<Inline> },
    Break,
}

/// One block of a body.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        children: Vec<Inline>,
    },
    List {
        ordered: bool,
        items: Vec<Vec<Inline>>,
    },
    Image {
        src: String,
        alt: String,
    },
}

/// Whether a destination is shaped like one: no control characters, no
/// whitespace, no backslash, not protocol-relative. The guide renderer's
/// rule (`guides::destination_shaped`), for the reasons it gives.
fn destination_shaped(raw: &str) -> bool {
    !raw.is_empty()
        && !raw.starts_with("//")
        && !raw.chars().any(|character| {
            character.is_control() || character.is_whitespace() || character == '\\'
        })
}

/// The scheme a destination names, lowercased.
fn scheme_of(raw: &str) -> Option<String> {
    let (scheme, _) = raw.split_once(':')?;
    let mut characters = scheme.chars();
    let first = characters.next()?;
    (first.is_ascii_alphabetic()
        && characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then(|| scheme.to_ascii_lowercase())
}

/// A link's destination if a mail may carry it: web and mail addresses.
fn allowed_href(raw: &str) -> Option<String> {
    let decoded = decode_entities(raw);
    let href = decoded.trim();
    if !destination_shaped(href) || href.chars().count() > URL_MAX_CHARS {
        return None;
    }
    matches!(
        scheme_of(href).as_deref(),
        Some("https" | "http" | "mailto")
    )
    .then(|| href.to_owned())
}

/// Whether a path names an uploaded mail picture: the prefix and a
/// 64-character lowercase hex hash, nothing else.
fn is_mail_image_path(raw: &str) -> bool {
    raw.strip_prefix(IMAGE_PATH).is_some_and(|handle| {
        handle.len() == 64
            && handle
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// A picture's source if a mail may show it: an uploaded mail picture, or
/// an HTTPS address.
fn allowed_src(raw: &str) -> Option<String> {
    let decoded = decode_entities(raw);
    let src = decoded.trim();
    if !destination_shaped(src) || src.chars().count() > URL_MAX_CHARS {
        return None;
    }
    (is_mail_image_path(src) || scheme_of(src).as_deref() == Some("https")).then(|| src.to_owned())
}

/// Whitespace runs as one space.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut spaced = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !spaced {
                out.push(' ');
            }
            spaced = true;
        } else {
            spaced = false;
            out.push(c);
        }
    }
    out
}

/// Lowers a run of inline nodes. Pictures met inline are hoisted into
/// `pictures`, to follow the block they were in.
fn inlines(nodes: &[Node], out: &mut Vec<Inline>, pictures: &mut Vec<Block>) {
    for node in nodes {
        match node {
            Node::Text(text) => push_text(out, &collapse(text)),
            Node::Element(element) => inline_element(element, out, pictures),
        }
    }
}

fn push_text(out: &mut Vec<Inline>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(Inline::Text(last)) = out.last_mut() {
        if last.ends_with(' ') && text.starts_with(' ') {
            last.push_str(text.trim_start_matches(' '));
        } else {
            last.push_str(text);
        }
    } else {
        out.push(Inline::Text(text.to_owned()));
    }
}

fn inline_element(element: &Element, out: &mut Vec<Inline>, pictures: &mut Vec<Block>) {
    match element.name.as_str() {
        "br" => out.push(Inline::Break),
        "img" => {
            if let Some(picture) = picture(element) {
                pictures.push(picture);
            }
        }
        "strong" | "b" => wrap(out, element, pictures, Inline::Strong),
        "em" | "i" => wrap(out, element, pictures, Inline::Em),
        "a" => {
            let mut children = Vec::new();
            inlines(&element.children, &mut children, pictures);
            match element.href.as_deref().and_then(allowed_href) {
                Some(href) if has_words(&children) => out.push(Inline::Link { href, children }),
                _ => out.extend(children),
            }
        }
        _ => inlines(&element.children, out, pictures),
    }
}

fn wrap(
    out: &mut Vec<Inline>,
    element: &Element,
    pictures: &mut Vec<Block>,
    make: fn(Vec<Inline>) -> Inline,
) {
    let mut children = Vec::new();
    inlines(&element.children, &mut children, pictures);
    if has_words(&children) {
        // Spaces at the edges of an emphasis belong outside it, so the
        // reduction of its own output is itself.
        let leading = starts_with_space(&children);
        let trailing = ends_with_space(&children);
        trim_start(&mut children);
        trim_end(&mut children);
        if leading {
            push_text(out, " ");
        }
        out.push(make(children));
        if trailing {
            push_text(out, " ");
        }
    } else {
        out.extend(children);
    }
}

fn picture(element: &Element) -> Option<Block> {
    let src = element.src.as_deref().and_then(allowed_src)?;
    let alt = collapse(&decode_entities(element.alt.as_deref().unwrap_or_default()))
        .trim()
        .to_owned();
    Some(Block::Image { src, alt })
}

fn has_words(nodes: &[Inline]) -> bool {
    nodes.iter().any(|node| match node {
        Inline::Text(text) => !text.trim().is_empty(),
        Inline::Strong(children) | Inline::Em(children) | Inline::Link { children, .. } => {
            has_words(children)
        }
        Inline::Break => false,
    })
}

fn starts_with_space(nodes: &[Inline]) -> bool {
    match nodes.first() {
        Some(Inline::Text(text)) => text.starts_with(' '),
        Some(Inline::Strong(children) | Inline::Em(children) | Inline::Link { children, .. }) => {
            starts_with_space(children)
        }
        Some(Inline::Break) | None => false,
    }
}

fn ends_with_space(nodes: &[Inline]) -> bool {
    match nodes.last() {
        Some(Inline::Text(text)) => text.ends_with(' '),
        Some(Inline::Strong(children) | Inline::Em(children) | Inline::Link { children, .. }) => {
            ends_with_space(children)
        }
        Some(Inline::Break) | None => false,
    }
}

/// Leading spaces and line breaks off a run.
fn trim_start(nodes: &mut Vec<Inline>) {
    loop {
        match nodes.first_mut() {
            Some(Inline::Break) => {
                nodes.remove(0);
            }
            Some(Inline::Text(text)) => {
                let kept = text.trim_start().to_owned();
                if kept.is_empty() {
                    nodes.remove(0);
                } else {
                    *text = kept;
                    return;
                }
            }
            Some(
                Inline::Strong(children) | Inline::Em(children) | Inline::Link { children, .. },
            ) => {
                trim_start(children);
                return;
            }
            None => return,
        }
    }
}

/// Trailing spaces and line breaks off a run.
fn trim_end(nodes: &mut Vec<Inline>) {
    loop {
        match nodes.last_mut() {
            Some(Inline::Break) => {
                nodes.pop();
            }
            Some(Inline::Text(text)) => {
                let kept = text.trim_end().to_owned();
                if kept.is_empty() {
                    nodes.pop();
                } else {
                    *text = kept;
                    return;
                }
            }
            Some(
                Inline::Strong(children) | Inline::Em(children) | Inline::Link { children, .. },
            ) => {
                trim_end(children);
                return;
            }
            None => return,
        }
    }
}

/// Closes the paragraph being gathered, if it holds any words, and appends
/// the pictures met inside it.
fn flush(run: &mut Vec<Inline>, pictures: &mut Vec<Block>, out: &mut Vec<Block>) {
    let mut taken = core::mem::take(run);
    trim_start(&mut taken);
    trim_end(&mut taken);
    if has_words(&taken) {
        out.push(Block::Paragraph(taken));
    }
    out.append(pictures);
}

/// Lowers a sequence of nodes into blocks.
fn blocks(nodes: &[Node], out: &mut Vec<Block>) {
    let mut run = Vec::new();
    let mut pictures = Vec::new();
    for node in nodes {
        let Node::Element(element) = node else {
            inlines(core::slice::from_ref(node), &mut run, &mut pictures);
            continue;
        };
        match element.name.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                flush(&mut run, &mut pictures, out);
                let mut children = Vec::new();
                inlines(&element.children, &mut children, &mut pictures);
                trim_start(&mut children);
                trim_end(&mut children);
                if has_words(&children) {
                    let level = if matches!(element.name.as_str(), "h1" | "h2") {
                        2
                    } else {
                        3
                    };
                    out.push(Block::Heading { level, children });
                }
                out.append(&mut pictures);
            }
            "ul" | "ol" => {
                flush(&mut run, &mut pictures, out);
                let mut items = Vec::new();
                list_items(&element.children, &mut items, &mut pictures);
                if !items.is_empty() {
                    out.push(Block::List {
                        ordered: element.name == "ol",
                        items,
                    });
                }
                out.append(&mut pictures);
            }
            "img" => {
                flush(&mut run, &mut pictures, out);
                if let Some(picture) = picture(element) {
                    out.push(picture);
                }
            }
            name if is_block(name) => {
                flush(&mut run, &mut pictures, out);
                blocks(&element.children, out);
            }
            _ => inline_element(element, &mut run, &mut pictures),
        }
    }
    flush(&mut run, &mut pictures, out);
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "blockquote"
            | "pre"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "main"
            | "aside"
            | "nav"
            | "figure"
            | "figcaption"
            | "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "dl"
            | "dt"
            | "dd"
            | "address"
            | "center"
            | "details"
            | "summary"
            | "li"
            | "hr"
    )
}

/// A list's items, each one line of inline content. A nested list's items
/// join the outer list: a mail client's indentation of a second level is the
/// part of a list least likely to survive.
fn list_items(nodes: &[Node], items: &mut Vec<Vec<Inline>>, pictures: &mut Vec<Block>) {
    for node in nodes {
        let Node::Element(element) = node else {
            continue;
        };
        match element.name.as_str() {
            "li" => {
                let mut line = Vec::new();
                let mut nested = Vec::new();
                for child in &element.children {
                    match child {
                        Node::Element(inner) if matches!(inner.name.as_str(), "ul" | "ol") => {
                            nested.push(inner);
                        }
                        Node::Element(inner) if is_block(&inner.name) => {
                            push_text(&mut line, " ");
                            inlines(&inner.children, &mut line, pictures);
                            push_text(&mut line, " ");
                        }
                        other @ (Node::Text(_) | Node::Element(_)) => {
                            inlines(core::slice::from_ref(other), &mut line, pictures);
                        }
                    }
                }
                trim_start(&mut line);
                trim_end(&mut line);
                if has_words(&line) {
                    items.push(line);
                }
                for inner in nested {
                    list_items(&inner.children, items, pictures);
                }
            }
            "ul" | "ol" => list_items(&element.children, items, pictures),
            _ => {}
        }
    }
}

fn lower(html: &str) -> Vec<Block> {
    let root = parse(html);
    let mut out = Vec::new();
    blocks(&root.children, &mut out);
    out
}

fn plain_inline(nodes: &[Inline], out: &mut String) {
    for node in nodes {
        match node {
            Inline::Text(text) => escape_text(text, out),
            Inline::Strong(children) => {
                out.push_str("<strong>");
                plain_inline(children, out);
                out.push_str("</strong>");
            }
            Inline::Em(children) => {
                out.push_str("<em>");
                plain_inline(children, out);
                out.push_str("</em>");
            }
            Inline::Link { href, children } => {
                out.push_str("<a href=\"");
                out.push_str(&escape_attribute(href));
                out.push_str("\">");
                plain_inline(children, out);
                out.push_str("</a>");
            }
            Inline::Break => out.push_str("<br>"),
        }
    }
}

/// The body as it is stored: the allow-listed elements only, text escaped,
/// every block holding words. Empty when nothing is written.
#[must_use]
pub fn sanitise_body(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    for block in lower(html) {
        match block {
            Block::Paragraph(children) => {
                out.push_str("<p>");
                plain_inline(&children, &mut out);
                out.push_str("</p>");
            }
            Block::Heading { level, children } => {
                let tag = if level == 2 { "h2" } else { "h3" };
                out.push('<');
                out.push_str(tag);
                out.push('>');
                plain_inline(&children, &mut out);
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
            }
            Block::List { ordered, items } => {
                let tag = if ordered { "ol" } else { "ul" };
                out.push('<');
                out.push_str(tag);
                out.push('>');
                for item in items {
                    out.push_str("<li>");
                    plain_inline(&item, &mut out);
                    out.push_str("</li>");
                }
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
            }
            Block::Image { src, alt } => {
                out.push_str("<img src=\"");
                out.push_str(&escape_attribute(&src));
                out.push_str("\" alt=\"");
                out.push_str(&escape_attribute(&alt));
                out.push_str("\">");
            }
        }
    }
    out
}

// ========================================================== the variables

/// One variable a body may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variable {
    Name,
    FirstName,
    Email,
    Org,
    Plan,
    Date,
    Link,
    Unsubscribe,
}

/// Longest first, so `@first_name` is never read as something shorter.
const VARIABLES: [(&str, Variable); 8] = [
    ("unsubscribe", Variable::Unsubscribe),
    ("first_name", Variable::FirstName),
    ("email", Variable::Email),
    ("name", Variable::Name),
    ("date", Variable::Date),
    ("link", Variable::Link),
    ("plan", Variable::Plan),
    ("org", Variable::Org),
];

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A run of text split into literal pieces and variables. A token counts
/// only where it stands alone: not after a word character or a dot (so an
/// address's domain is not a variable) and not followed by a word character.
fn tokens(text: &str) -> Vec<Result<&str, Variable>> {
    let mut out = Vec::new();
    let mut literal_from = 0;
    let mut previous: Option<char> = None;
    for (at, c) in text.char_indices() {
        let after = previous;
        previous = Some(c);
        if c != '@' || after.is_some_and(|p| is_word(p) || p == '.' || p == '@') {
            continue;
        }
        let Some(rest) = text.get(at + 1..) else {
            continue;
        };
        let Some((word, variable)) = VARIABLES.iter().find(|(word, _)| {
            rest.strip_prefix(word)
                .is_some_and(|tail| !tail.chars().next().is_some_and(is_word))
        }) else {
            continue;
        };
        if let Some(literal) = text.get(literal_from..at).filter(|l| !l.is_empty()) {
            out.push(Ok(literal));
        }
        out.push(Err(*variable));
        literal_from = at + 1 + word.len();
    }
    if let Some(literal) = text.get(literal_from..).filter(|l| !l.is_empty()) {
        out.push(Ok(literal));
    }
    out
}

/// Who one mail is for, as the variables name them.
#[derive(Debug, Clone, Copy)]
pub struct Personal<'a> {
    /// The identity service's display name, if it holds one.
    pub name: Option<&'a str>,
    pub email: &'a str,
    pub org: &'a str,
    pub plan: &'a str,
    /// The send date, as [`long_date`] writes it.
    pub date: &'a str,
    /// The absolute unsubscribe link.
    pub unsubscribe_url: &'a str,
}

/// What one campaign says.
#[derive(Debug, Clone, Copy)]
pub struct Letter<'a> {
    pub subject: &'a str,
    /// The stored, sanitised body.
    pub body_html: &'a str,
    pub link_url: Option<&'a str>,
    pub link_label: Option<&'a str>,
}

/// One rendered mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub subject: String,
    pub html: String,
    pub text: String,
}

/// A display name made safe to place in a line: controls and invisible
/// format characters dropped, whitespace collapsed, length bounded.
fn clean_name(raw: &str) -> String {
    let kept: String = raw
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|&c| {
            !c.is_control()
                && !matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}' | '\u{00AD}')
        })
        .collect();
    collapse(kept.trim()).chars().take(NAME_MAX_CHARS).collect()
}

/// A resolved value for each variable but the two that are links.
struct Values {
    name: String,
    first_name: String,
    email: String,
    org: String,
    plan: String,
    date: String,
}

impl Values {
    fn of(personal: &Personal<'_>) -> Self {
        let name = personal.name.map(clean_name).filter(|n| !n.is_empty());
        let first_name = name
            .as_deref()
            .and_then(|n| n.split(' ').next())
            .map(str::to_owned);
        Self {
            name: name.unwrap_or_else(|| "there".to_owned()),
            first_name: first_name.unwrap_or_else(|| "there".to_owned()),
            email: personal.email.to_owned(),
            org: clean_name(personal.org),
            plan: personal.plan.to_owned(),
            date: personal.date.to_owned(),
        }
    }

    fn text(&self, variable: Variable) -> Option<&str> {
        match variable {
            Variable::Name => Some(&self.name),
            Variable::FirstName => Some(&self.first_name),
            Variable::Email => Some(&self.email),
            Variable::Org => Some(&self.org),
            Variable::Plan => Some(&self.plan),
            Variable::Date => Some(&self.date),
            Variable::Link | Variable::Unsubscribe => None,
        }
    }
}

/// The subject with its text variables filled; a link variable in a subject
/// is left as written, because a subject cannot carry a link.
fn fill_subject(subject: &str, values: &Values) -> String {
    let mut out = String::with_capacity(subject.len());
    for token in tokens(subject) {
        match token {
            Ok(literal) => out.push_str(literal),
            Err(variable) => {
                if let Some(value) = values.text(variable) {
                    out.push_str(value);
                } else {
                    out.push('@');
                    out.push_str(
                        VARIABLES
                            .iter()
                            .find(|(_, v)| *v == variable)
                            .map_or("", |(word, _)| word),
                    );
                }
            }
        }
    }
    collapse(&out).trim().to_owned()
}

/// Everything a rendering pass needs, so the recursive writers take one
/// argument for it.
struct Pass<'a> {
    values: Values,
    link: Option<&'a str>,
    label: &'a str,
    unsubscribe: &'a str,
    origin: &'a str,
}

impl Pass<'_> {
    /// An absolute address for a site-relative one.
    fn absolute(&self, path: &str) -> String {
        if path.starts_with('/') {
            let mut out = self.origin.trim_end_matches('/').to_owned();
            out.push_str(path);
            out
        } else {
            path.to_owned()
        }
    }
}

fn anchor(href: &str, label: &str, out: &mut String) {
    out.push_str("<a href=\"");
    out.push_str(&escape_attribute(href));
    out.push_str("\" style=\"color:");
    out.push_str(PRIMARY);
    out.push_str(";text-decoration:underline;\">");
    escape_text(label, out);
    out.push_str("</a>");
}

fn styled_text(text: &str, pass: &Pass<'_>, in_link: bool, out: &mut String) {
    for token in tokens(text) {
        match token {
            Ok(literal) => escape_text(literal, out),
            Err(Variable::Link) => match pass.link {
                Some(link) if !in_link => anchor(link, pass.label, out),
                _ => escape_text(pass.label, out),
            },
            Err(Variable::Unsubscribe) => {
                if in_link {
                    escape_text("unsubscribe", out);
                } else {
                    anchor(pass.unsubscribe, "unsubscribe", out);
                }
            }
            Err(variable) => escape_text(pass.values.text(variable).unwrap_or_default(), out),
        }
    }
}

fn styled_inline(nodes: &[Inline], pass: &Pass<'_>, in_link: bool, out: &mut String) {
    for node in nodes {
        match node {
            Inline::Text(text) => styled_text(text, pass, in_link, out),
            Inline::Strong(children) => {
                out.push_str("<strong style=\"font-weight:700;\">");
                styled_inline(children, pass, in_link, out);
                out.push_str("</strong>");
            }
            Inline::Em(children) => {
                out.push_str("<em style=\"font-style:italic;\">");
                styled_inline(children, pass, in_link, out);
                out.push_str("</em>");
            }
            Inline::Link { href, children } => {
                out.push_str("<a href=\"");
                out.push_str(&escape_attribute(href));
                out.push_str("\" style=\"color:");
                out.push_str(PRIMARY);
                out.push_str(";text-decoration:underline;\">");
                styled_inline(children, pass, true, out);
                out.push_str("</a>");
            }
            Inline::Break => out.push_str("<br>"),
        }
    }
}

/// Whether a paragraph is the button: `@link` and nothing else.
fn is_button(children: &[Inline]) -> bool {
    matches!(children, [Inline::Text(text)] if text.trim() == "@link")
}

fn button(href: &str, label: &str, out: &mut String) {
    use core::fmt::Write as _;
    let _unused: core::fmt::Result = write!(
        out,
        "<table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"margin:4px 0 20px;\"><tr>\
<td align=\"center\" bgcolor=\"{PRIMARY}\" style=\"border-radius:999px;background-color:{PRIMARY};padding:14px 28px;\">\
<a href=\"{href}\" style=\"display:inline-block;font-family:{BODY};font-size:15px;font-weight:600;line-height:1.2;color:{ON_FILL};text-decoration:none;mso-padding-alt:0;\">",
        href = escape_attribute(href),
    );
    escape_text(label, out);
    out.push_str("</a></td></tr></table>");
}

fn styled_blocks(blocks: &[Block], pass: &Pass<'_>, out: &mut String) {
    use core::fmt::Write as _;
    for block in blocks {
        match block {
            Block::Paragraph(children) if is_button(children) => {
                if let Some(link) = pass.link {
                    button(link, pass.label, out);
                }
            }
            Block::Paragraph(children) => {
                let _unused: core::fmt::Result = write!(
                    out,
                    "<p style=\"margin:0;padding-bottom:16px;font-family:{BODY};font-size:15px;line-height:1.6;color:{TEXT};\">"
                );
                styled_inline(children, pass, false, out);
                out.push_str("</p>");
            }
            Block::Heading { level, children } => {
                let (tag, size) = if *level == 2 { ("h2", 22) } else { ("h3", 18) };
                let _unused: core::fmt::Result = write!(
                    out,
                    "<{tag} style=\"margin:0;padding:4px 0 12px;font-family:{DISPLAY};font-size:{size}px;font-weight:600;line-height:1.3;color:{PRIMARY};\">"
                );
                styled_inline(children, pass, false, out);
                let _unused: core::fmt::Result = write!(out, "</{tag}>");
            }
            Block::List { ordered, items } => {
                let tag = if *ordered { "ol" } else { "ul" };
                let _unused: core::fmt::Result = write!(
                    out,
                    "<{tag} style=\"margin:0 0 16px;padding-left:22px;font-family:{BODY};font-size:15px;line-height:1.6;color:{TEXT};\">"
                );
                for item in items {
                    out.push_str("<li style=\"margin:0 0 6px;\">");
                    styled_inline(item, pass, false, out);
                    out.push_str("</li>");
                }
                let _unused: core::fmt::Result = write!(out, "</{tag}>");
            }
            Block::Image { src, alt } => {
                let _unused: core::fmt::Result = write!(
                    out,
                    "<p style=\"margin:0;padding-bottom:16px;\"><img src=\"{}\" alt=\"{}\" style=\"display:block;max-width:100%;height:auto;border:0;border-radius:8px;\"></p>",
                    escape_attribute(&pass.absolute(src)),
                    escape_attribute(alt),
                );
            }
        }
    }
}

fn text_inline(nodes: &[Inline], pass: &Pass<'_>, out: &mut String) {
    for node in nodes {
        match node {
            Inline::Text(text) => {
                for token in tokens(text) {
                    match token {
                        Ok(literal) => out.push_str(literal),
                        Err(Variable::Link) => {
                            out.push_str(pass.label);
                            if let Some(link) = pass.link {
                                out.push_str(" (");
                                out.push_str(link);
                                out.push(')');
                            }
                        }
                        Err(Variable::Unsubscribe) => {
                            out.push_str("unsubscribe (");
                            out.push_str(pass.unsubscribe);
                            out.push(')');
                        }
                        Err(variable) => {
                            out.push_str(pass.values.text(variable).unwrap_or_default());
                        }
                    }
                }
            }
            Inline::Strong(children) | Inline::Em(children) => text_inline(children, pass, out),
            Inline::Link { href, children } => {
                text_inline(children, pass, out);
                out.push_str(" (");
                out.push_str(href);
                out.push(')');
            }
            Inline::Break => out.push('\n'),
        }
    }
}

fn text_blocks(blocks: &[Block], pass: &Pass<'_>, out: &mut String) {
    for block in blocks {
        match block {
            Block::Paragraph(children) if is_button(children) => {
                if let Some(link) = pass.link {
                    out.push_str(pass.label);
                    out.push_str(":\n");
                    out.push_str(link);
                    out.push_str("\n\n");
                }
            }
            Block::Paragraph(children) | Block::Heading { children, .. } => {
                text_inline(children, pass, out);
                out.push_str("\n\n");
            }
            Block::List { ordered, items } => {
                for (index, item) in items.iter().enumerate() {
                    if *ordered {
                        out.push_str(&(index + 1).to_string());
                        out.push_str(". ");
                    } else {
                        out.push_str("- ");
                    }
                    text_inline(item, pass, out);
                    out.push('\n');
                }
                out.push('\n');
            }
            Block::Image { alt, .. } => {
                if !alt.is_empty() {
                    out.push('[');
                    out.push_str(alt);
                    out.push_str("]\n\n");
                }
            }
        }
    }
}

/// "29 September 2026", in UTC.
#[must_use]
pub fn long_date(at: Timestamp) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let (year, month, day) = crate::time::civil_from_days(at.0.div_euclid(86_400_000));
    let name = usize::try_from(month - 1)
        .ok()
        .and_then(|index| MONTHS.get(index))
        .copied()
        .unwrap_or_default();
    format!("{day} {name} {year}")
}

/// The year the footer's copyright names.
fn year_of(date: &str) -> &str {
    date.rsplit(' ').next().unwrap_or_default()
}

/// One mail, as a pure function of what the campaign says, who it is for,
/// and the origin its pictures, faces and links are served from. An empty
/// origin leaves site paths relative, which is what the console's preview
/// frame wants.
#[must_use]
pub fn render(letter: &Letter<'_>, personal: &Personal<'_>, origin: &str) -> Rendered {
    use core::fmt::Write as _;
    let origin = origin.trim_end_matches('/');
    let pass = Pass {
        values: Values::of(personal),
        link: letter.link_url.filter(|url| !url.is_empty()),
        label: letter
            .link_label
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .unwrap_or(DEFAULT_LINK_LABEL),
        unsubscribe: personal.unsubscribe_url,
        origin,
    };
    let body = lower(letter.body_html);
    let subject = fill_subject(letter.subject, &pass.values);

    let mut content = String::with_capacity(letter.body_html.len() * 2);
    styled_blocks(&body, &pass, &mut content);
    let mut text = String::with_capacity(letter.body_html.len());
    text_blocks(&body, &pass, &mut text);
    let preheader: String = collapse(&text).trim().chars().take(140).collect();

    let site = escape_attribute(origin);
    let home = if origin.is_empty() { "/" } else { origin };
    let year = year_of(personal.date);
    let unsubscribe = escape_attribute(personal.unsubscribe_url);
    let mut html = String::with_capacity(content.len() + 4_096);
    let _unused: core::fmt::Result = write!(
        html,
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width,initial-scale=1" />
    <meta name="color-scheme" content="light only" />
    <meta name="supported-color-schemes" content="light only" />
    <meta name="x-apple-disable-message-reformatting" />
    <title>{title}</title>
    <style>
      :root {{ color-scheme: light only; supported-color-schemes: light only; }}
      @font-face {{ font-family: Poppins; font-weight: 600; font-style: normal; src: url('{site}/fonts/poppins-600-latin.woff2') format('woff2'); }}
      @font-face {{ font-family: Inter; font-weight: 400 700; font-style: normal; src: url('{site}/fonts/inter-400-700-latin.woff2') format('woff2'); }}
      @media only screen and (max-width: 620px) {{
        .tam-column {{ width: 100% !important; }}
        .tam-pad {{ padding: 24px !important; }}
      }}
    </style>
  </head>
  <body bgcolor="{GROUND}" style="margin:0;padding:0;width:100%;background-color:{GROUND};color:{TEXT};">
    <div style="display:none;max-height:0;max-width:0;overflow:hidden;opacity:0;font-size:1px;line-height:1px;color:{GROUND};">{preheader}{filler}</div>
    <table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="{GROUND}" style="width:100%;background-color:{GROUND};">
      <tr>
        <td align="center" bgcolor="{GROUND}" style="padding:32px 12px;background-color:{GROUND};">
          <table role="presentation" class="tam-column" width="600" cellpadding="0" cellspacing="0" border="0" bgcolor="{SURFACE}" style="width:600px;max-width:600px;background-color:{SURFACE};border:1px solid {LINE};border-top:4px solid {ACCENT};border-radius:14px;">
            <tr>
              <td class="tam-pad" bgcolor="{SURFACE}" style="padding:28px 32px 0;background-color:{SURFACE};border-radius:13px 13px 0 0;line-height:0;">
                <img src="{site}/email/teachouse-wordmark.png" width="176" height="43" alt="Teachouse" style="display:block;width:176px;height:43px;border:0;font-family:{DISPLAY};font-size:22px;font-weight:600;line-height:43px;color:{PRIMARY};" />
              </td>
            </tr>
            <tr>
              <td class="tam-pad" bgcolor="{SURFACE}" style="padding:28px 32px 16px;background-color:{SURFACE};">
{content}
              </td>
            </tr>
            <tr>
              <td align="center" bgcolor="{SURFACE}" style="padding:24px;background-color:{SURFACE};border-top:1px solid {LINE};border-radius:0 0 13px 13px;">
          <p style="margin:0;padding-bottom:4px;font-family:{BODY};font-size:12px;line-height:1.5;color:{MUTED};">&copy; {year} Teachouse</p>
          <p style="margin:0;padding-bottom:4px;font-family:{BODY};font-size:12px;line-height:1.5;color:{MUTED};">Auckland 1072, New Zealand</p>
          <p style="margin:0;padding-bottom:12px;font-family:{BODY};font-size:12px;line-height:1.5;color:{MUTED};"><a href="{home}" style="color:{MUTED};text-decoration:underline;">Powered by PLE Group</a></p>
          <p style="margin:0;font-family:{BODY};font-size:12px;line-height:1.5;color:{MUTED};">You get this because you have a Teachouse account. <a href="{unsubscribe}" style="color:{MUTED};text-decoration:underline;">Unsubscribe</a></p>
              </td>
            </tr>
          </table>
        </td>
      </tr>
    </table>
  </body>
</html>
"#,
        title = escape_attribute(&subject),
        preheader = escape_attribute(&preheader),
        filler = "&#847;&zwnj;&nbsp;".repeat(30),
        home = escape_attribute(home),
    );

    let _unused: core::fmt::Result = write!(
        text,
        "--\n\u{a9} {year} Teachouse\nAuckland 1072, New Zealand\nPowered by PLE Group: {home}\nUnsubscribe: {}\n",
        personal.unsubscribe_url
    );
    Rendered {
        subject,
        html,
        text,
    }
}

/// The values the preview is drawn with.
fn sample(date: &str) -> Personal<'_> {
    Personal {
        name: Some("Ana Ruiz"),
        email: "ana@example.com",
        org: "Ana's Classroom",
        plan: "Sync",
        date,
        unsubscribe_url: "#unsubscribe",
    }
}

// =============================================================== checks

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

fn storage_fault(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    state.internal(&error.to_string())
}

/// A draft made storable: the body reduced, the link checked, the subject
/// bounded. What is stored is what this answers, never the request.
fn checked(draft: &MailDraft) -> Result<MailDraft, APIError> {
    let subject = collapse(draft.subject.trim());
    if subject.is_empty() {
        return Err(validation("Give the email a subject."));
    }
    if subject.chars().count() > SUBJECT_MAX_CHARS {
        return Err(validation("A subject is at most 200 characters."));
    }
    if draft.body_html.len() > BODY_MAX_BYTES {
        return Err(validation("This email is too long to send."));
    }
    let body_html = sanitise_body(&draft.body_html);
    if body_html.is_empty() {
        return Err(validation("Write something in the email."));
    }
    let link_url = match draft
        .link_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        None => None,
        Some(raw) => Some(
            allowed_href(raw)
                .filter(|href| scheme_of(href).as_deref() == Some("https"))
                .ok_or_else(|| validation("The button link must start with https://."))?,
        ),
    };
    let uses_link = tokens(&body_html).contains(&Err(Variable::Link));
    if uses_link && link_url.is_none() {
        return Err(validation(
            "The email has a @link button; add the address it opens.",
        ));
    }
    let link_label = draft
        .link_label
        .as_deref()
        .map(|label| collapse(label.trim()))
        .filter(|label| !label.is_empty());
    if link_label
        .as_ref()
        .is_some_and(|label| label.chars().count() > LABEL_MAX_CHARS)
    {
        return Err(validation("A button label is at most 60 characters."));
    }
    Ok(MailDraft {
        subject,
        body_html,
        link_url,
        link_label,
    })
}

fn parse_id(raw: &str) -> Result<Uuid, APIError> {
    uuid::Uuid::parse_str(raw)
        .map(|parsed| Uuid(*parsed.as_bytes()))
        .map_err(|_| validation("the identifier is not a UUID"))
}

fn new_id() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}

/// The name a plan is sold under.
fn plan_name(plan: Plan) -> &'static str {
    tam_limits::PLANS
        .iter()
        .find(|row| row.id == plan)
        .map_or("Look", |row| row.name)
}

// =============================================================== audience

/// Who a filter reaches, and the count of who it leaves out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Selection {
    pub recipients: Vec<NewRecipient>,
    pub count: MailAudienceCount,
}

/// The audience as a pure function of the sellers, the plan each
/// organisation holds, and who the operators are.
///
/// A seller the segment matches is left out when they are an operator (and
/// the filter says so), when they opted out, and when they have no sign-in
/// identity; each is counted once, under the first of those reasons.
#[must_use]
pub(crate) fn select(
    members: &[AudienceMember],
    plans: &HashMap<OrgId, Plan>,
    operators: &HashSet<UserId>,
    audience: MailAudience,
) -> Selection {
    let mut count = MailAudienceCount {
        recipients: 0,
        sellers: 0,
        operators_excluded: 0,
        opted_out: 0,
        no_sign_in: 0,
    };
    let mut recipients = Vec::new();
    for member in members {
        let plan = plans.get(&member.org).copied().unwrap_or(Plan::Free);
        if !audience.segment.admits(plan) {
            continue;
        }
        count.sellers += 1;
        if audience.exclude_operators && operators.contains(&member.user) {
            count.operators_excluded += 1;
            continue;
        }
        if member.opted_out {
            count.opted_out += 1;
            continue;
        }
        let Some(subject) = member.auth_subject else {
            count.no_sign_in += 1;
            continue;
        };
        count.recipients += 1;
        recipients.push(NewRecipient {
            user: member.user,
            auth_subject: subject,
            org_name: member.org_name.clone(),
            plan: plan_name(plan).to_owned(),
        });
    }
    Selection { recipients, count }
}

async fn selection(state: &AppState, audience: MailAudience) -> Result<Selection, APIError> {
    let now = (state.wall)();
    let plans = BackofficeRepo::new(crate::admin::backoffice(state)?)
        .plans(now)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let members = MailCampaignRepo::new(state.pool.clone())
        .audience_members()
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let operators: HashSet<UserId> = OperatorRepo::new(state.pool.clone())
        .list()
        .await
        .map_err(|error| storage_fault(state, &error))?
        .into_iter()
        .filter(|record| record.revoked_at.is_none())
        .map(|record| record.user)
        .collect();
    Ok(select(&members, &plans, &operators, audience))
}

// ================================================================ routes

#[derive(Debug, Deserialize)]
pub struct AudienceParams {
    segment: Segment,
    #[serde(default = "yes")]
    exclude_operators: bool,
    #[serde(default = "yes")]
    verified_only: bool,
}

/// How many sellers a filter reaches before anything is sent.
pub(crate) async fn audience_count(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Query(params): Query<AudienceParams>,
) -> Result<Json<MailAudienceCount>, APIError> {
    let audience = MailAudience {
        segment: params.segment,
        exclude_operators: params.exclude_operators,
        verified_only: params.verified_only,
    };
    Ok(Json(selection(&state, audience).await?.count))
}

/// The draft rendered in the wrapper with sample values, for the console's
/// preview frame.
pub(crate) async fn preview(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Json(draft): Json<MailDraft>,
) -> Result<Json<MailPreviewView>, APIError> {
    let draft = checked(&draft)?;
    let date = long_date((state.wall)());
    let rendered = render(&letter_of(&draft), &sample(&date), "");
    Ok(Json(MailPreviewView {
        subject: rendered.subject,
        html: rendered.html,
    }))
}

fn letter_of(draft: &MailDraft) -> Letter<'_> {
    Letter {
        subject: &draft.subject,
        body_html: &draft.body_html,
        link_url: draft.link_url.as_deref(),
        link_label: draft.link_label.as_deref(),
    }
}

fn label_of(raw: &str) -> String {
    let label = clean_name(raw);
    if label.is_empty() {
        "an admin".to_owned()
    } else {
        label
    }
}

/// Queues one mail of the draft to the operator sending it.
pub(crate) async fn send_test(
    State(state): State<AppState>,
    operator: OperatorContext,
    Json(body): Json<TestMailBody>,
) -> Result<(StatusCode, Json<MailQueuedView>), APIError> {
    let draft = checked(&body.draft)?;
    let repo = MailCampaignRepo::new(state.pool.clone());
    let me = repo
        .audience_members()
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .into_iter()
        .find(|member| member.user == operator.user)
        .ok_or_else(|| missing("We can't find your account."))?;
    let subject = me
        .auth_subject
        .ok_or_else(|| validation("Your account has no sign-in email to send a test to."))?;
    let plan = match state.backoffice.clone() {
        Some(pool) => BackofficeRepo::new(pool)
            .plans((state.wall)())
            .await
            .map_err(|error| storage_fault(&state, &error))?
            .get(&me.org)
            .copied()
            .unwrap_or(Plan::Free),
        None => Plan::Free,
    };
    repo.purge_finished_tests(operator.user)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let id = new_id();
    let audience = serde_json::json!({
        "segment": "all", "exclude_operators": false, "verified_only": false,
    });
    repo.create(
        &NewCampaign {
            id,
            subject: &draft.subject,
            body_html: &draft.body_html,
            link_url: draft.link_url.as_deref(),
            link_label: draft.link_label.as_deref(),
            audience: &audience,
            test: true,
            created_by: operator.user,
            created_by_label: &label_of(&body.created_by_label),
            at: (state.wall)(),
        },
        &[NewRecipient {
            user: me.user,
            auth_subject: subject,
            org_name: me.org_name,
            plan: plan_name(plan).to_owned(),
        }],
    )
    .await
    .map_err(|error| storage_fault(&state, &error))?;
    Ok((StatusCode::ACCEPTED, Json(MailQueuedView { id })))
}

/// Writes a campaign and queues one row per recipient.
pub(crate) async fn create_campaign(
    State(state): State<AppState>,
    operator: OperatorContext,
    Json(body): Json<NewCampaignBody>,
) -> Result<(StatusCode, Json<MailCampaignDetail>), APIError> {
    let draft = checked(&body.draft)?;
    let chosen = selection(&state, body.audience).await?;
    if chosen.recipients.is_empty() {
        return Err(validation(
            "Nobody matches these filters, so nothing was sent.",
        ));
    }
    let id = new_id();
    let audience = serde_json::to_value(body.audience)
        .map_err(|error| state.internal(&format!("the audience did not serialise: {error}")))?;
    let repo = MailCampaignRepo::new(state.pool.clone());
    repo.create(
        &NewCampaign {
            id,
            subject: &draft.subject,
            body_html: &draft.body_html,
            link_url: draft.link_url.as_deref(),
            link_label: draft.link_label.as_deref(),
            audience: &audience,
            test: false,
            created_by: operator.user,
            created_by_label: &label_of(&body.created_by_label),
            at: (state.wall)(),
        },
        &chosen.recipients,
    )
    .await
    .map_err(|error| storage_fault(&state, &error))?;
    eprintln!(
        "tam-api: operator {} queued mail campaign {} to {} recipients",
        operator.user.0.to_hyphenated(),
        id.to_hyphenated(),
        chosen.recipients.len()
    );
    Ok((StatusCode::CREATED, Json(detail(&state, id).await?)))
}

fn audience_of(value: &serde_json::Value) -> MailAudience {
    serde_json::from_value(value.clone()).unwrap_or(MailAudience {
        segment: Segment::All,
        exclude_operators: true,
        verified_only: true,
    })
}

fn summary_of(record: &CampaignRecord) -> MailCampaignSummary {
    MailCampaignSummary {
        id: record.id,
        subject: record.subject.clone(),
        audience: audience_of(&record.audience),
        counts: MailCountsView::of(record.counts),
        created_at: record.created_at,
        created_by: record.created_by,
        created_by_label: record.created_by_label.clone(),
        test: record.test,
        deleted_at: record.deleted_at,
    }
}

fn recipient_of(record: RecipientRecord) -> MailRecipientView {
    MailRecipientView {
        user: record.user,
        auth_subject: Some(record.auth_subject),
        org_name: record.org_name,
        plan: record.plan,
        status: record.status,
        attempts: record.attempts,
        provider_id: record.provider_id,
        error: record.error,
        updated_at: record.updated_at,
    }
}

async fn detail(state: &AppState, id: Uuid) -> Result<MailCampaignDetail, APIError> {
    let repo = MailCampaignRepo::new(state.pool.clone());
    let record = repo
        .get(id)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("We can't find that email."))?;
    let recipients = repo
        .recipients(id)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(MailCampaignDetail {
        summary: summary_of(&record),
        draft: record.body_html.clone().map(|body_html| MailDraft {
            subject: record.subject.clone(),
            body_html,
            link_url: record.link_url.clone(),
            link_label: record.link_label.clone(),
        }),
        recipients: recipients.into_iter().map(recipient_of).collect(),
    })
}

/// The campaign log, newest first.
pub(crate) async fn list_campaigns(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<MailCampaignsView>, APIError> {
    let records = MailCampaignRepo::new(state.pool.clone())
        .list()
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(MailCampaignsView {
        campaigns: records.iter().map(summary_of).collect(),
    }))
}

/// One campaign with its recipients.
pub(crate) async fn campaign_detail(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, id)): Path<(String, String)>,
) -> Result<Json<MailCampaignDetail>, APIError> {
    Ok(Json(detail(&state, parse_id(&id)?).await?))
}

/// Puts a campaign's failed recipients back in the queue.
pub(crate) async fn retry_campaign(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, id)): Path<(String, String)>,
) -> Result<Json<MailCampaignDetail>, APIError> {
    let id = parse_id(&id)?;
    let current = detail(&state, id).await?;
    if current.summary.deleted_at.is_some() {
        return Err(validation(
            "This email was deleted; there is nothing to send again.",
        ));
    }
    MailCampaignRepo::new(state.pool.clone())
        .retry_failed(id, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(detail(&state, id).await?))
}

/// Deletes a campaign's body and recipient rows, keeping the log line.
pub(crate) async fn delete_campaign(
    State(state): State<AppState>,
    operator: OperatorContext,
    Path((_version, id)): Path<(String, String)>,
) -> Result<Json<MailCampaignDetail>, APIError> {
    let id = parse_id(&id)?;
    let deleted = MailCampaignRepo::new(state.pool.clone())
        .delete(id, operator.user, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if !deleted {
        let _exists = detail(&state, id).await?;
        return Err(validation("This email is already deleted."));
    }
    eprintln!(
        "tam-api: operator {} deleted mail campaign {}; its log line is kept",
        operator.user.0.to_hyphenated(),
        id.to_hyphenated()
    );
    Ok(Json(detail(&state, id).await?))
}

fn hex_of(hash: tam_types::ContentHash) -> String {
    use core::fmt::Write as _;
    let mut hex = String::with_capacity(64);
    for byte in hash.0 {
        let _unused: core::fmt::Result = write!(hex, "{byte:02x}");
    }
    hex
}

fn blob_fault(state: &AppState, error: &BlobError) -> APIError {
    state.internal(&error.to_string())
}

/// Stores a picture for a mail body and lists it as public.
///
/// Sealed under the platform organisation like a guide's picture, uncharged
/// for the reason that one is, and listed in `mail_image` so the public
/// route below serves it: a mail client fetches pictures with no session.
pub(crate) async fn upload_image(
    State(state): State<AppState>,
    operator: OperatorContext,
    body: Bytes,
) -> Result<(StatusCode, Json<MailImageView>), APIError> {
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
            APIErrorEntry::new("A picture is a PNG, JPEG, GIF or WebP.")
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
    MailCampaignRepo::new(state.pool.clone())
        .record_image(hash, operator.user, now)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let handle = hex_of(hash);
    let mut url = IMAGE_PATH.to_owned();
    url.push_str(&handle);
    Ok((StatusCode::CREATED, Json(MailImageView { handle, url })))
}

/// The bytes of one mail picture, to anyone: a mail client has no session.
/// A handle not uploaded for mail answers 404, guide pictures included.
pub(crate) async fn image(
    State(state): State<AppState>,
    Path((_version, handle)): Path<(String, String)>,
) -> Result<([(header::HeaderName, &'static str); 3], Vec<u8>), APIError> {
    let hash = parse_hash(&handle).ok_or_else(|| missing("We can't find that picture."))?;
    let listed = MailCampaignRepo::new(state.pool.clone())
        .image_listed(hash)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let Some(blobs) = state.blobs.clone().filter(|_| listed) else {
        return Err(missing("We can't find that picture."));
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
            fault @ (BlobError::Storage(_) | BlobError::Store(_) | BlobError::Crypto(_)) => {
                blob_fault(&state, &fault)
            }
        })?;
    image_answer(bytes)
}

// =========================================================== unsubscribe

#[derive(Debug, Deserialize)]
pub struct UnsubscribeParams {
    #[serde(default)]
    t: String,
}

/// A small page in the mail's own colours, for the unsubscribe link.
fn page(title: &str, body: &str, form: Option<&str>) -> Html<String> {
    let mut action = String::new();
    if let Some(token) = form {
        action = format!(
            "<form method=\"post\" action=\"{UNSUBSCRIBE_PATH}?t={}\" style=\"margin:24px 0 0;\">\
<button type=\"submit\" style=\"border:0;border-radius:999px;background:{PRIMARY};color:{ON_FILL};font:600 15px {BODY};padding:14px 28px;cursor:pointer;\">Unsubscribe</button></form>",
            escape_attribute(token)
        );
    }
    let mut escaped_title = String::new();
    escape_text(title, &mut escaped_title);
    let mut escaped_body = String::new();
    escape_text(body, &mut escaped_body);
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"robots\" content=\"noindex\"><title>{escaped_title} · Teachouse</title></head>\
<body style=\"margin:0;background:{GROUND};color:{TEXT};font-family:{BODY};\"><main style=\"max-width:520px;margin:48px auto;padding:32px;background:{SURFACE};border:1px solid {LINE};border-top:4px solid {ACCENT};border-radius:14px;\">\
<img src=\"/email/teachouse-wordmark.png\" width=\"176\" height=\"43\" alt=\"Teachouse\" style=\"display:block;margin-bottom:24px;\">\
<h1 style=\"margin:0 0 12px;font:600 22px {DISPLAY};color:{PRIMARY};\">{escaped_title}</h1><p style=\"margin:0;font-size:15px;line-height:1.6;\">{escaped_body}</p>{action}</main></body></html>"
    ))
}

fn token_of(raw: &str) -> Option<Uuid> {
    uuid::Uuid::parse_str(raw.trim())
        .ok()
        .map(|parsed| Uuid(*parsed.as_bytes()))
}

/// What the unsubscribe link opens. A page with a button rather than the
/// change itself: link scanners and previewers open every link in a mail,
/// and a GET that unsubscribed would unsubscribe people who never clicked.
pub(crate) async fn unsubscribe_page(Query(params): Query<UnsubscribeParams>) -> Response {
    if token_of(&params.t).is_none() {
        return (
            StatusCode::NOT_FOUND,
            page(
                "This link doesn't work",
                "Open Settings in Teachouse to choose which emails you get.",
                None,
            ),
        )
            .into_response();
    }
    page(
        "Stop news from Teachouse?",
        "You will stop getting news and tips from us. Emails about your own resources and account still arrive.",
        Some(params.t.trim()),
    )
    .into_response()
}

/// The unsubscribe itself: the form above, and a mail client's one-click
/// `List-Unsubscribe-Post` (RFC 8058), which posts
/// `List-Unsubscribe=One-Click` to the same address. Idempotent.
pub(crate) async fn unsubscribe(
    State(state): State<AppState>,
    Query(params): Query<UnsubscribeParams>,
    headers: HeaderMap,
) -> Result<Response, APIError> {
    let Some(token) = token_of(&params.t) else {
        return Ok((
            StatusCode::NOT_FOUND,
            page(
                "This link doesn't work",
                "Open Settings in Teachouse to choose which emails you get.",
                None,
            ),
        )
            .into_response());
    };
    let known = MailCampaignRepo::new(state.pool.clone())
        .opt_out_by_token(token)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if !known {
        return Ok((
            StatusCode::NOT_FOUND,
            page(
                "This link doesn't work",
                "Open Settings in Teachouse to choose which emails you get.",
                None,
            ),
        )
            .into_response());
    }
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("application/json"));
    if wants_json {
        return Ok(Json(serde_json::json!({ "unsubscribed": true })).into_response());
    }
    Ok(page(
        "You're unsubscribed",
        "We won't send you news and tips any more. You can turn them back on in Settings, under Notifications.",
        None,
    )
    .into_response())
}

/// The seller's own switch, read beside `notify_email`.
pub(crate) async fn marketing_preference(
    state: &AppState,
    context: &OrgContext,
    wanted: Option<bool>,
) -> Result<bool, APIError> {
    let repo = MailCampaignRepo::new(state.pool.clone());
    let answer = match wanted {
        Some(wanted) => {
            repo.set_marketing_email(context.org, context.user, wanted)
                .await
        }
        None => repo.marketing_email(context.org, context.user).await,
    };
    answer
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("We can't find your account."))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use tam_limits::Plan;
    use tam_storage::AudienceMember;
    use tam_types::{OrgId, Timestamp, UserId, Uuid};

    use super::{
        checked, long_date, render, sanitise_body, select, Letter, MailAudience, MailDraft,
        Personal, Segment,
    };

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn person<'a>(name: Option<&'a str>, org: &'a str) -> Personal<'a> {
        Personal {
            name,
            email: "ana@example.test",
            org,
            plan: "Sync",
            date: "29 September 2026",
            unsubscribe_url: "https://app.example.test/v1/mail/unsubscribe?t=abc",
        }
    }

    fn letter(body: &str) -> Letter<'_> {
        Letter {
            subject: "Hello @first_name",
            body_html: body,
            link_url: Some("https://teachouse.io/new"),
            link_label: Some("See what's new"),
        }
    }

    #[test]
    fn the_body_keeps_the_allow_list_and_nothing_else() {
        let dirty = format!(
            "<h1>Big</h1><h4>Small</h4><p style=\"color:red\" onclick=\"x()\">Hi <b>there</b> <i>you</i> \
             <a href=\"javascript:alert(1)\">bad</a> <a href=\"https://ok.test/?a=1&amp;b=2\">good</a></p>\
             <script>alert(1)</script><img src=\"x\" onerror=\"alert(1)\">\
             <img src=\"/v1/mail/images/{HASH}\" alt=\"A &quot;chart&quot;\">\
             <img src=\"https://cdn.test/p.png\"><img src=\"data:image/png;base64,AAAA\">\
             <ul><li>one<ul><li>two</li></ul></li></ul><table><tr><td>cell</td></tr></table>"
        );
        let clean = sanitise_body(&dirty);
        assert_eq!(
            clean,
            format!(
                "<h2>Big</h2><h3>Small</h3><p>Hi <strong>there</strong> <em>you</em> bad \
                 <a href=\"https://ok.test/?a=1&amp;b=2\">good</a></p>\
                 <img src=\"/v1/mail/images/{HASH}\" alt=\"A &quot;chart&quot;\">\
                 <img src=\"https://cdn.test/p.png\" alt=\"\">\
                 <ul><li>one</li><li>two</li></ul><p>cell</p>"
            )
        );
        assert_eq!(sanitise_body(&clean), clean, "the reduction is idempotent");
    }

    #[test]
    fn variables_are_filled_and_every_value_is_escaped() {
        let body = sanitise_body(
            "<p>Hi @first_name, from @org on @plan (@email) on @date.</p><p>@link</p><p>Bye @unsubscribe</p>",
        );
        let hostile = person(Some("<script>alert(1)</script> Smith"), "A & B <Co>");
        let mail = render(&letter(&body), &hostile, "https://app.example.test");
        assert!(mail.html.contains(
            "Hi &lt;script&gt;alert(1)&lt;/script&gt;, from A &amp; B &lt;Co&gt; on Sync (ana@example.test) on 29 September 2026."
        ));
        assert!(
            !mail.html.contains("<script>"),
            "no markup from a value survives"
        );
        assert!(
            mail.html.contains("href=\"https://teachouse.io/new\""),
            "the button"
        );
        assert!(
            mail.html.contains(">See what&#39;s new</a>")
                || mail.html.contains(">See what's new</a>")
        );
        assert!(mail
            .html
            .contains("href=\"https://app.example.test/v1/mail/unsubscribe?t=abc\""));
        assert_eq!(mail.subject, "Hello <script>alert(1)</script>");
        assert!(mail
            .html
            .contains("<title>Hello &lt;script&gt;alert(1)&lt;/script&gt;</title>"));
        assert!(mail
            .text
            .contains("See what's new:\nhttps://teachouse.io/new"));
        assert!(mail
            .text
            .contains("Unsubscribe: https://app.example.test/v1/mail/unsubscribe?t=abc"));
    }

    #[test]
    fn a_token_counts_only_where_it_stands_alone() {
        let body = sanitise_body("<p>Write to hi@org.test or @organisation, @name_x, @name.</p>");
        let mail = render(&letter(&body), &person(Some("Ana Ruiz"), "Org"), "");
        assert!(mail
            .html
            .contains("Write to hi@org.test or @organisation, @name_x, Ana Ruiz."));
    }

    #[test]
    fn a_missing_name_reads_there() {
        let body = sanitise_body("<p>Hi @first_name (@name)</p>");
        let mail = render(&letter(&body), &person(None, "Org"), "");
        assert!(mail.html.contains("Hi there (there)"));
        let blank = render(&letter(&body), &person(Some(" \u{200B} "), "Org"), "");
        assert!(blank.html.contains("Hi there (there)"));
    }

    #[test]
    fn relative_pictures_become_absolute_on_the_origin() {
        let body = sanitise_body(&format!(
            "<p><img src=\"/v1/mail/images/{HASH}\" alt=\"\"></p>"
        ));
        let mail = render(
            &letter(&body),
            &person(None, "Org"),
            "https://app.example.test/",
        );
        assert!(mail.html.contains(&format!(
            "src=\"https://app.example.test/v1/mail/images/{HASH}\""
        )));
        assert!(mail
            .html
            .contains("src=\"https://app.example.test/email/teachouse-wordmark.png\""));
    }

    #[test]
    fn a_draft_is_refused_where_it_cannot_be_sent() {
        let draft = |subject: &str, body: &str, link: Option<&str>| MailDraft {
            subject: subject.to_owned(),
            body_html: body.to_owned(),
            link_url: link.map(str::to_owned),
            link_label: None,
        };
        assert!(checked(&draft(" ", "<p>x</p>", None)).is_err());
        assert!(checked(&draft(&"s".repeat(201), "<p>x</p>", None)).is_err());
        assert!(checked(&draft(&"s".repeat(200), "<p>x</p>", None)).is_ok());
        assert!(checked(&draft("s", "<script>x</script>", None)).is_err());
        assert!(checked(&draft("s", "<p>@link</p>", None)).is_err());
        assert!(checked(&draft("s", "<p>@link</p>", Some("http://x.test"))).is_err());
        assert!(checked(&draft("s", "<p>@link</p>", Some("javascript:x"))).is_err());
        let ok = checked(&draft(
            "  Two   words ",
            "<p>@link</p>",
            Some("https://x.test"),
        ))
        .expect("a sendable draft");
        assert_eq!(ok.subject, "Two words");
    }

    #[test]
    fn the_date_is_written_long() {
        assert_eq!(long_date(Timestamp(1_790_640_000_000)), "29 September 2026");
        assert_eq!(long_date(Timestamp(0)), "1 January 1970");
    }

    fn member(byte: u8, org: u8, subject: bool, opted_out: bool) -> AudienceMember {
        AudienceMember {
            user: UserId(Uuid([byte; 16])),
            auth_subject: subject.then_some(Uuid([byte ^ 0xFF; 16])),
            org: OrgId(Uuid([org; 16])),
            org_name: format!("org {org}"),
            opted_out,
        }
    }

    #[test]
    fn the_audience_filters_count_who_is_in_and_why_the_rest_are_out() {
        let members = [
            member(1, 1, true, false),  // free
            member(2, 2, true, false),  // starter
            member(3, 3, true, false),  // sync
            member(4, 4, true, false),  // studio, an operator
            member(5, 3, true, true),   // sync, opted out
            member(6, 2, false, false), // starter, never signed in
        ];
        let plans = HashMap::from([
            (OrgId(Uuid([2; 16])), Plan::Starter),
            (OrgId(Uuid([3; 16])), Plan::Subscriber),
            (OrgId(Uuid([4; 16])), Plan::Studio),
        ]);
        let operators = HashSet::from([UserId(Uuid([4; 16]))]);
        let count = |segment, exclude_operators| {
            select(
                &members,
                &plans,
                &operators,
                MailAudience {
                    segment,
                    exclude_operators,
                    verified_only: true,
                },
            )
        };
        let all = count(Segment::All, true);
        assert_eq!(all.count.sellers, 6);
        assert_eq!(all.count.recipients, 3);
        assert_eq!(all.count.operators_excluded, 1);
        assert_eq!(all.count.opted_out, 1);
        assert_eq!(all.count.no_sign_in, 1);
        assert_eq!(count(Segment::All, false).count.recipients, 4);
        assert_eq!(count(Segment::Free, true).count.recipients, 1);
        assert_eq!(count(Segment::Paid, true).count.recipients, 2);
        assert_eq!(count(Segment::Paid, false).count.recipients, 3);
        assert_eq!(count(Segment::Starter, true).count.recipients, 1);
        assert_eq!(count(Segment::Starter, true).count.sellers, 2);
        let sync = count(Segment::Subscriber, true);
        assert_eq!((sync.count.sellers, sync.count.recipients), (2, 1));
        assert_eq!(sync.recipients[0].plan, "Sync");
        assert_eq!(count(Segment::Studio, true).count.recipients, 0);
        assert_eq!(count(Segment::Studio, false).count.recipients, 1);
    }
}
