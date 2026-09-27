//! The rich-text description's allow-list, enforced where a body is written.
//!
//! The console's editor writes HTML, and only the eight elements every
//! marketplace description field renders alike: `p`, `br`, `strong`, `em`,
//! `ul`, `ol`, `li` and `a` with a web or mail `href`. The browser reduces
//! what it sends to that list (`web/src/lib/rich-text.ts`), and this module
//! reduces it again, because a request is whatever its sender chose to send.
//! The two are the same rules written twice, and `sanitise_html` is
//! idempotent over either's output.
//!
//! Applied on write rather than on projection: an HTML body imported from a
//! marketplace keeps whatever that marketplace let its seller write until the
//! seller edits it here, and the TPT projection carries it through as it
//! always has.

use pulldown_cmark::{html::push_html, Options, Parser};

enum Node {
    Text(String),
    Element(Element),
}

/// The default is the nameless root every parse starts from.
#[derive(Default)]
struct Element {
    name: String,
    href: Option<String>,
    children: Vec<Node>,
}

/// Elements with no closing tag.
const VOID: &[&str] = &[
    "br", "img", "hr", "input", "meta", "link", "wbr", "source", "area", "col",
];

/// Elements whose content is not text a reader sees, dropped whole.
const OPAQUE: &[&str] = &[
    "script", "style", "template", "noscript", "iframe", "object", "textarea", "title", "head",
    "svg", "math", "select",
];

/// Elements that start a new block wherever they appear.
const BLOCKS: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "pre",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "nav",
    "figure",
    "figcaption",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "dl",
    "dt",
    "dd",
    "address",
    "center",
    "details",
    "summary",
    "li",
    "ul",
    "ol",
    "hr",
];

const NAMED_ENTITIES: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", '\u{a0}'),
    ("ndash", '–'),
    ("mdash", '—'),
    ("hellip", '…'),
    ("lsquo", '‘'),
    ("rsquo", '’'),
    ("ldquo", '“'),
    ("rdquo", '”'),
    ("bull", '•'),
    ("middot", '·'),
    ("copy", '©'),
    ("reg", '®'),
    ("trade", '™'),
    ("pound", '£'),
    ("euro", '€'),
];

fn is_block(name: &str) -> bool {
    BLOCKS.contains(&name)
}

fn is_list(name: &str) -> bool {
    name == "ul" || name == "ol"
}

/// Named and numeric references decoded; an unknown one stays as written.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('&') {
        out.push_str(before);
        let decoded = after
            .char_indices()
            .take(9)
            .find(|&(_, c)| c == ';')
            .and_then(|(end, _)| {
                let (body, remaining) = after.split_at_checked(end)?;
                let character = if let Some(digits) = body.strip_prefix('#') {
                    let code = match digits.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => digits.parse::<u32>().ok(),
                    };
                    code.filter(|&code| code > 0).and_then(char::from_u32)
                } else {
                    NAMED_ENTITIES
                        .iter()
                        .find(|(name, _)| *name == body)
                        .map(|(_, c)| *c)
                };
                character.zip(remaining.strip_prefix(';'))
            });
        if let Some((c, remaining)) = decoded {
            out.push(c);
            rest = remaining;
        } else {
            out.push('&');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn escape_text(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}

fn escape_attribute(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// A link a reader can follow and nothing else: web and mail addresses.
fn safe_href(raw: &str) -> Option<String> {
    let decoded = decode_entities(raw);
    let href: String = decoded
        .trim()
        .chars()
        .flat_map(|c| {
            if c.is_whitespace() {
                "%20".chars().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect();
    let lower = href.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:"))
        .then_some(href)
}

/// The `href` among a tag's attributes, as written.
fn href_of(attributes: &str) -> Option<String> {
    let bytes = attributes.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        while at < bytes.len() && (bytes[at].is_ascii_whitespace() || bytes[at] == b'/') {
            at += 1;
        }
        let start = at;
        while at < bytes.len()
            && !bytes[at].is_ascii_whitespace()
            && !matches!(bytes[at], b'=' | b'>' | b'/' | b'"' | b'\'')
        {
            at += 1;
        }
        if at == start {
            at += 1;
            continue;
        }
        let name = attributes.get(start..at).unwrap_or_default();
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let mut value = "";
        if at < bytes.len() && bytes[at] == b'=' {
            at += 1;
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            if at < bytes.len() && matches!(bytes[at], b'"' | b'\'') {
                let quote = bytes[at];
                let from = at + 1;
                at = from;
                while at < bytes.len() && bytes[at] != quote {
                    at += 1;
                }
                value = attributes.get(from..at).unwrap_or_default();
                at += 1;
            } else {
                let from = at;
                while at < bytes.len() && !bytes[at].is_ascii_whitespace() {
                    at += 1;
                }
                value = attributes.get(from..at).unwrap_or_default();
            }
        }
        if name.eq_ignore_ascii_case("href") {
            return Some(value.to_owned());
        }
    }
    None
}

/// A forgiving tree: unknown closing tags are ignored, unclosed ones end with
/// their parent, and a new paragraph or list item closes the open one the way
/// a browser would. Text is decoded.
fn parse(html: &str) -> Element {
    fn close_to(stack: &mut Vec<Element>, name: &str, fence: &[&str]) {
        if let Some(found) = (1..stack.len())
            .rev()
            .take_while(|&at| stack[at].name == name || !fence.contains(&stack[at].name.as_str()))
            .find(|&at| stack[at].name == name)
        {
            close_above(stack, found);
        }
    }
    fn push_text(stack: &mut [Element], text: &str) {
        if let Some(top) = stack.last_mut() {
            top.children.push(Node::Text(decode_entities(text)));
        }
    }

    let mut stack = vec![Element::default()];
    let bytes = html.as_bytes();
    let mut at = 0;
    while let Some(tail) = html.get(at..).filter(|tail| !tail.is_empty()) {
        let Some((text, tag)) = tail
            .find('<')
            .and_then(|offset| tail.split_at_checked(offset))
        else {
            push_text(&mut stack, tail);
            break;
        };
        let lt = at + text.len();
        if !text.is_empty() {
            push_text(&mut stack, text);
        }
        if let Some(comment) = tag.strip_prefix("<!--") {
            at = comment
                .find("-->")
                .map_or(html.len(), |end| lt + 4 + end + 3);
            continue;
        }
        let closing = bytes.get(lt + 1) == Some(&b'/');
        let name_start = lt + 1 + usize::from(closing);
        if !bytes.get(name_start).is_some_and(u8::is_ascii_alphabetic) {
            if matches!(bytes.get(lt + 1), Some(b'!' | b'?')) {
                at = tag.find('>').map_or(html.len(), |end| lt + end + 1);
            } else {
                push_text(&mut stack, "<");
                at = lt + 1;
            }
            continue;
        }
        let mut name_end = name_start;
        while bytes
            .get(name_end)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-')
        {
            name_end += 1;
        }
        // The tag runs to the first `>` outside a quoted attribute value.
        let mut end = name_end;
        let mut quote: Option<u8> = None;
        while end < bytes.len() {
            let b = bytes[end];
            match quote {
                Some(q) if b == q => quote = None,
                None if b == b'"' || b == b'\'' => quote = Some(b),
                None if b == b'>' => break,
                Some(_) | None => {}
            }
            end += 1;
        }
        if end >= bytes.len() {
            // An unclosed tag swallows the rest: its text was never visible.
            break;
        }
        let name = html
            .get(name_start..name_end)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let attributes = html.get(name_end..end).unwrap_or_default();
        at = end + 1;
        if closing {
            let fence: &[&str] = if name == "li" { &["ul", "ol"] } else { &[] };
            close_to(&mut stack, &name, fence);
            continue;
        }
        if OPAQUE.contains(&name.as_str()) {
            at = closing_tag(html, at, &name).unwrap_or(html.len());
            continue;
        }
        if is_block(&name) {
            close_to(
                &mut stack,
                "p",
                &["li", "ul", "ol", "blockquote", "td", "th"],
            );
        }
        if name == "li" {
            close_to(&mut stack, "li", &["ul", "ol"]);
        }
        let element = Element {
            href: (name == "a").then(|| href_of(attributes)).flatten(),
            name,
            children: Vec::new(),
        };
        if VOID.contains(&element.name.as_str()) || attributes.trim_end().ends_with('/') {
            if let Some(top) = stack.last_mut() {
                top.children.push(Node::Element(element));
            }
        } else {
            stack.push(element);
        }
    }
    close_to_root(&mut stack)
}

/// Closes every open element above `depth`, each into the one below it.
fn close_above(stack: &mut Vec<Element>, depth: usize) {
    while stack.len() > depth {
        let Some(done) = stack.pop() else {
            return;
        };
        let Some(parent) = stack.last_mut() else {
            return;
        };
        parent.children.push(Node::Element(done));
    }
}

fn close_to_root(stack: &mut Vec<Element>) -> Element {
    close_above(stack, 1);
    stack.pop().unwrap_or_default()
}

/// Where the text after `</name>` starts, matched case-insensitively.
fn closing_tag(html: &str, from: usize, name: &str) -> Option<usize> {
    let lower = html.get(from..)?.to_ascii_lowercase();
    let needle = format!("</{name}");
    let mut search = 0;
    while let Some(found) = lower.get(search..).and_then(|rest| rest.find(&needle)) {
        let after = search + found + needle.len();
        let rest = lower.get(after..)?;
        let trimmed = rest.trim_start();
        if trimmed.starts_with('>') {
            return Some(from + after + (rest.len() - trimmed.len()) + 1);
        }
        search = after;
    }
    None
}

/// Whitespace runs as one space, and no-break spaces as ordinary ones.
fn collapse(text: &str, out: &mut String) {
    let mut spaced = false;
    for c in text.chars() {
        if matches!(c, '\t' | '\n' | '\r' | '\u{c}' | ' ') {
            if !spaced {
                out.push(' ');
            }
            spaced = true;
        } else {
            spaced = false;
            out.push(if c == '\u{a0}' { ' ' } else { c });
        }
    }
}

/// Whether a fragment of our own output holds any visible words.
fn has_text(html: &str) -> bool {
    let mut text = String::new();
    let mut inside = false;
    for c in html.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => text.push(c),
            _ => {}
        }
    }
    !decode_entities(&text).trim().is_empty()
}

/// Leading and trailing spaces and line breaks off a run of inline HTML, and
/// the doubled spaces two adjacent text nodes leave.
fn tidy_inline(html: &str) -> String {
    let mut single = String::with_capacity(html.len());
    for c in html.chars() {
        if !(c == ' ' && single.ends_with(' ')) {
            single.push(c);
        }
    }
    let mut tidy = single.replace(" <br>", "<br>").replace("<br> ", "<br>");
    loop {
        if let Some(rest) = tidy.strip_prefix(' ').or_else(|| tidy.strip_prefix("<br>")) {
            tidy = rest.to_owned();
        } else if let Some(rest) = tidy.strip_suffix(' ').or_else(|| tidy.strip_suffix("<br>")) {
            tidy = rest.to_owned();
        } else {
            return tidy;
        }
    }
}

fn inline(node: &Node, out: &mut String) {
    let element = match node {
        Node::Text(text) => {
            let mut collapsed = String::with_capacity(text.len());
            collapse(text, &mut collapsed);
            escape_text(&collapsed, out);
            return;
        }
        Node::Element(element) => element,
    };
    let inner = || {
        let mut inner = String::new();
        for child in &element.children {
            inline(child, &mut inner);
        }
        inner
    };
    match element.name.as_str() {
        "br" => out.push_str("<br>"),
        "strong" | "b" => wrap("strong", &inner(), out),
        "em" | "i" => wrap("em", &inner(), out),
        "a" => {
            let words = inner();
            match element.href.as_deref().and_then(safe_href) {
                Some(href) if has_text(&words) => {
                    out.push_str("<a href=\"");
                    out.push_str(&escape_attribute(&href));
                    out.push_str("\">");
                    out.push_str(&words);
                    out.push_str("</a>");
                }
                _ => out.push_str(&words),
            }
        }
        "td" | "th" => {
            out.push(' ');
            out.push_str(&inner());
            out.push(' ');
        }
        name if is_block(name) => {
            out.push_str("<br>");
            out.push_str(&inner());
            out.push_str("<br>");
        }
        _ => out.push_str(&inner()),
    }
}

/// An inline element around its content, with the spaces at its edges moved
/// outside it so a Markdown rendering of the same text still emphasises.
fn wrap(tag: &str, content: &str, out: &mut String) {
    if !has_text(content) {
        out.push_str(content);
        return;
    }
    let mut body = content;
    while let Some(rest) = body.strip_prefix(' ').or_else(|| body.strip_prefix("<br>")) {
        body = rest;
    }
    let lead = content.strip_suffix(body).unwrap_or_default();
    let unled = body;
    while let Some(rest) = body.strip_suffix(' ').or_else(|| body.strip_suffix("<br>")) {
        body = rest;
    }
    let trail = unled.strip_prefix(body).unwrap_or_default();
    out.push_str(lead);
    out.push('<');
    out.push_str(tag);
    out.push('>');
    out.push_str(body);
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
    out.push_str(trail);
}

fn flush_paragraphs(run: &mut String, out: &mut String) {
    let joined = run.replace("<br> ", "<br>");
    for piece in joined.split("<br><br>") {
        let tidy = tidy_inline(piece);
        if has_text(&tidy) {
            out.push_str("<p>");
            out.push_str(&tidy);
            out.push_str("</p>");
        }
    }
    run.clear();
}

/// A sequence of block children as paragraphs and lists.
fn blocks(children: &[Node], out: &mut String) {
    let mut run = String::new();
    for child in children {
        match child {
            Node::Element(element) if is_list(&element.name) => {
                flush_paragraphs(&mut run, out);
                out.push_str(&list_of(element));
            }
            Node::Element(element) if element.name == "tr" => {
                flush_paragraphs(&mut run, out);
                for cell in &element.children {
                    inline(cell, &mut run);
                }
                flush_paragraphs(&mut run, out);
            }
            Node::Element(element) if is_block(&element.name) => {
                flush_paragraphs(&mut run, out);
                blocks(&element.children, out);
            }
            Node::Text(_) | Node::Element(_) => inline(child, &mut run),
        }
    }
    flush_paragraphs(&mut run, out);
}

fn list_of(list: &Element) -> String {
    let mut items: Vec<String> = Vec::new();
    let mut loose = String::new();
    let flush_loose = |loose: &mut String, items: &mut Vec<String>| {
        let tidy = tidy_inline(loose);
        if has_text(&tidy) {
            items.push(format!("<li>{tidy}</li>"));
        }
        loose.clear();
    };
    for child in &list.children {
        match child {
            Node::Element(element) if element.name == "li" => {
                flush_loose(&mut loose, &mut items);
                let item = item_of(element);
                if !item.is_empty() {
                    items.push(format!("<li>{item}</li>"));
                }
            }
            // A list directly inside a list is how a browser indents: it
            // belongs to the item before it.
            Node::Element(element) if is_list(&element.name) => {
                flush_loose(&mut loose, &mut items);
                let nested = list_of(element);
                if nested.is_empty() {
                    continue;
                }
                match items.pop() {
                    Some(previous) => {
                        let open = previous.strip_suffix("</li>").unwrap_or(&previous);
                        items.push(format!("{open}{nested}</li>"));
                    }
                    None => items.push(format!("<li>{nested}</li>")),
                }
            }
            Node::Text(_) | Node::Element(_) => inline(child, &mut loose),
        }
    }
    flush_loose(&mut loose, &mut items);
    if items.is_empty() {
        String::new()
    } else {
        format!("<{0}>{1}</{0}>", list.name, items.concat())
    }
}

fn item_of(item: &Element) -> String {
    let mut words = String::new();
    let mut nested = String::new();
    for child in &item.children {
        match child {
            Node::Element(element) if is_list(&element.name) => {
                nested.push_str(&list_of(element));
            }
            // A paragraph inside an item is a loose list's; its lines stay.
            Node::Element(element) if is_block(&element.name) => {
                words.push_str("<br>");
                for grandchild in &element.children {
                    inline(grandchild, &mut words);
                }
                words.push_str("<br>");
            }
            Node::Text(_) | Node::Element(_) => inline(child, &mut words),
        }
    }
    let mut tidy = tidy_inline(&words);
    while tidy.contains("<br><br>") {
        tidy = tidy.replace("<br><br>", "<br>");
    }
    if !has_text(&tidy) {
        tidy.clear();
    }
    tidy + &nested
}

/// The description as the editor and the server both keep it: the eight
/// allowed elements, no attribute but a web or mail `href`, text escaped,
/// every paragraph and item holding words. Empty when nothing is written.
#[must_use]
pub fn sanitise_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    blocks(&parse(html).children, &mut out);
    out
}

/// A Markdown body as sanitised HTML, rendered under the rules the TPT
/// projection renders one with (CommonMark plus tables and strikethrough):
/// what a Markdown description becomes on a product whose body is HTML.
#[must_use]
pub fn markdown_to_html(markdown: &str) -> String {
    let mut rendered = String::new();
    push_html(
        &mut rendered,
        Parser::new_ext(
            markdown,
            Options::ENABLE_TABLES.union(Options::ENABLE_STRIKETHROUGH),
        ),
    );
    sanitise_html(&rendered)
}

#[cfg(test)]
mod tests {
    use super::{markdown_to_html, sanitise_html};

    #[test]
    fn keeps_the_eight_elements_exactly() {
        let clean = "<p>A <strong>bold</strong> and <em>quiet</em> word<br>on two lines.</p>\
                     <ul><li>One</li><li>Two</li></ul><ol><li>First</li></ol>\
                     <p><a href=\"https://example.com/a?b=1&amp;c=2\">a link</a></p>";
        assert_eq!(sanitise_html(clean), clean);
    }

    #[test]
    fn drops_scripts_handlers_styles_and_unsafe_links_but_keeps_the_words() {
        let dirty = "<p onclick=\"x()\" style=\"color:red\">Hi <script>alert(1)</script>\
                     <span class=\"c\">there</span></p>\
                     <p><a href=\"javascript:alert(1)\">click</a> <img src=x onerror=alert(1)></p>\
                     <style>p{}</style><iframe src=\"https://evil\"></iframe>\
                     <p><a href=' JavaScript:alert(1)'>again</a><svg><script>x</script></svg></p>";
        assert_eq!(
            sanitise_html(dirty),
            "<p>Hi there</p><p>click</p><p>again</p>"
        );
    }

    #[test]
    fn a_quoted_angle_bracket_does_not_end_the_tag() {
        assert_eq!(
            sanitise_html("<a title=\"a > b\" href=\"https://x.test\">x</a>"),
            "<p><a href=\"https://x.test\">x</a></p>"
        );
    }

    #[test]
    fn reads_a_browsers_b_i_and_div_as_strong_em_and_paragraphs() {
        assert_eq!(
            sanitise_html(
                "First line<div><b>Bold</b> <i>it</i></div><div><br></div><div>Last</div>"
            ),
            "<p>First line</p><p><strong>Bold</strong> <em>it</em></p><p>Last</p>"
        );
    }

    #[test]
    fn headings_and_rows_become_paragraphs() {
        assert_eq!(
            sanitise_html("<h2>Includes</h2><table><tr><td>a</td><td>b</td></tr></table>"),
            "<p>Includes</p><p>a b</p>"
        );
    }

    #[test]
    fn nothing_visible_is_empty() {
        assert_eq!(
            sanitise_html("<p><br></p><p>&nbsp; </p><ul><li></li></ul>"),
            ""
        );
    }

    #[test]
    fn text_that_looks_like_markup_stays_escaped() {
        assert_eq!(
            sanitise_html("<p>1 &lt; 2 &amp; &lt;b&gt; 3 < 4</p>"),
            "<p>1 &lt; 2 &amp; &lt;b&gt; 3 &lt; 4</p>"
        );
    }

    #[test]
    fn an_indented_list_hangs_off_the_item_above() {
        assert_eq!(
            sanitise_html("<ul><li>a</li><ul><li>b</li></ul></ul>"),
            "<ul><li>a<ul><li>b</li></ul></li></ul>"
        );
    }

    #[test]
    fn is_idempotent() {
        let once = sanitise_html("<div>x<b>y </b>z<ul><li><p>z</p><p>w</p></li></ul></div>");
        assert_eq!(once, "<p>x<strong>y</strong> z</p><ul><li>z<br>w</li></ul>");
        assert_eq!(sanitise_html(&once), once);
    }

    #[test]
    fn markdown_renders_into_the_allow_list() {
        assert_eq!(
            markdown_to_html("## Pack\n\nA **bold** word.\n\n- one\n- two\n\n1. first"),
            "<p>Pack</p><p>A <strong>bold</strong> word.</p><ul><li>one</li><li>two</li></ul>\
             <ol><li>first</li></ol>"
        );
    }
}
