//! The description Tes will accept, projected from whatever the source wrote.
//!
//! The 2026-08-29 probe showed the API takes `descriptionRawType: html` on a
//! draft, but the seller's draft page then shows the markup verbatim and the
//! publish check refuses the text the founder read on 2026-09-19 for two
//! reasons the API never named: longer than 3,000 characters, and carrying
//! emoji. So a TPT body -- HTML, emoji-strewn, often longer -- is rendered to
//! plain text here, once, and posted as `md`, which plain text is a subset
//! of. The three rules are Tes's as observed on its own draft page, not a
//! house style.

use tam_types::CopyFormat;

/// Tes's draft-page limit on the description, in the units its own counter
/// uses: JavaScript string length, which is UTF-16 code units.
pub const DESCRIPTION_LIMIT: usize = 3_000;

/// The body as Tes accepts it: plain text, no emoji, within the limit.
#[must_use]
pub fn tes_description(body: &str, format: CopyFormat) -> String {
    let text = match format {
        CopyFormat::Html => render_html(body),
        CopyFormat::Markdown => body.to_owned(),
    };
    truncate(&tidy(&strip_emoji(&text)), DESCRIPTION_LIMIT)
}

/// Tags that end a line for a reader, whatever the renderer would do inside
/// them; everything else is inline and vanishes without a trace. `li` opens
/// with a marker so a list stays a list once its bullets are gone.
fn render_html(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some((before, after)) = rest.split_once('<') {
        out.push_str(before);
        let Some((tag, remaining)) = after.split_once('>') else {
            // An unclosed `<` swallows the rest: text inside a truncated tag
            // was never visible either.
            rest = "";
            break;
        };
        rest = remaining;
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .flat_map(char::to_lowercase)
            .collect();
        let closing = tag.starts_with('/');
        match (name.as_str(), closing) {
            ("li", false) => out.push_str("\n- "),
            // An item or row ends where the next begins; the list as a whole
            // ends a line so the paragraph after it does not fuse.
            ("li" | "tr", true) | ("ul" | "ol" | "table", false) => {}
            (
                "ul" | "ol" | "table" | "br" | "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5"
                | "h6" | "blockquote" | "pre" | "hr" | "section" | "article" | "header" | "footer",
                _,
            ) => out.push('\n'),
            ("td" | "th", _) => out.push(' '),
            _ => {}
        }
    }
    out.push_str(rest);
    decode_entities(&out)
}

/// The named entities a description actually carries, plus numeric
/// references. An unrecognised entity stays verbatim rather than vanishing.
fn decode_entities(text: &str) -> String {
    const NAMED: [(&str, char); 8] = [
        ("amp", '&'),
        ("lt", '<'),
        ("gt", '>'),
        ("quot", '"'),
        ("apos", '\''),
        ("nbsp", ' '),
        ("ndash", '–'),
        ("mdash", '—'),
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('&') {
        out.push_str(before);
        let entity = after
            .bytes()
            .take(9)
            .position(|byte| byte == b';')
            .and_then(|end| after.split_at_checked(end + 1))
            .and_then(|(encoded, remaining)| {
                encoded.strip_suffix(';').map(|body| (body, remaining))
            });
        let Some((body, remaining)) = entity else {
            out.push('&');
            rest = after;
            continue;
        };
        rest = remaining;
        if let Some((_, decoded)) = NAMED.iter().find(|(name, _)| *name == body) {
            out.push(*decoded);
        } else if let Some(decoded) = body.strip_prefix('#').and_then(|digits| {
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse::<u32>().ok()?,
            };
            char::from_u32(code)
        }) {
            out.push(decoded);
        } else {
            out.push('&');
            out.push_str(body);
            out.push(';');
        }
    }
    out.push_str(rest);
    out
}

/// Whether Tes's counter would call this an emoji: the pictographic blocks,
/// the joiners and selectors that compose them, and the keycap and flag
/// pieces. Ordinary punctuation and symbols stay.
const fn is_emoji(c: char) -> bool {
    matches!(
        c as u32,
        0x200D            // zero width joiner
        | 0x20E3          // combining enclosing keycap
        | 0xFE0E..=0xFE0F // variation selectors
        | 0x2600..=0x27BF // miscellaneous symbols, dingbats
        | 0x2B00..=0x2BFF // arrows and shapes used as emoji
        | 0x1F000..=0x1FAFF // mahjong through symbols and pictographs extended-A
        | 0xE0020..=0xE007F // tag characters (flag sequences)
    )
}

fn strip_emoji(text: &str) -> String {
    text.chars().filter(|c| !is_emoji(*c)).collect()
}

/// Lines trimmed, runs of blank lines folded to one, spaces folded.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_pending = false;
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.is_empty() {
            blank_pending = !out.is_empty();
            continue;
        }
        if blank_pending {
            out.push_str("\n\n");
        } else if !out.is_empty() {
            out.push('\n');
        }
        blank_pending = false;
        out.push_str(&words.join(" "));
    }
    out
}

/// Cut at the last whitespace before the limit, measured in UTF-16 units.
fn truncate(text: &str, limit: usize) -> String {
    let mut units = 0;
    let mut cut = None;
    for (index, c) in text.char_indices() {
        units += c.len_utf16();
        if units > limit {
            cut = Some(index);
            break;
        }
    }
    let Some(cut) = cut else {
        return text.to_owned();
    };
    let head = text.get(..cut).unwrap_or(text);
    let at = head.rfind(char::is_whitespace).unwrap_or(cut);
    head.get(..at).unwrap_or(head).trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::{tes_description, DESCRIPTION_LIMIT};
    use tam_types::CopyFormat;

    #[test]
    fn html_becomes_readable_plain_text() {
        let body = "<p>Fractions &amp; decimals 📐</p><p><strong>Includes:</strong></p>\
                    <ul><li>Worksheet</li><li>Answers</li></ul><p>Enjoy!&nbsp;🎉</p>";
        assert_eq!(
            tes_description(body, CopyFormat::Html),
            "Fractions & decimals\n\nIncludes:\n- Worksheet\n- Answers\n\nEnjoy!"
        );
    }

    #[test]
    fn markdown_keeps_its_text_and_loses_its_emoji() {
        assert_eq!(
            tes_description("Level 2 ⭐️ pack — **new**", CopyFormat::Markdown),
            "Level 2 pack — **new**"
        );
    }

    #[test]
    fn the_limit_cuts_on_a_word_and_counts_like_the_draft_page() {
        let word = "twelve chars ";
        let long = word.repeat(300);
        let cut = tes_description(&long, CopyFormat::Markdown);
        assert!(cut.encode_utf16().count() <= DESCRIPTION_LIMIT);
        assert!(
            cut.split(' ').all(|w| w == "twelve" || w == "chars"),
            "cut on a word boundary: {cut:?}"
        );
        assert!(cut.encode_utf16().count() > DESCRIPTION_LIMIT - word.len());
    }
}
