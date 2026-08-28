//! The form-page scrape: the CakePHP token triple, the double-submit CSRF
//! pair, the published AWS access key id and, on an edit render, the existing
//! thumbnail handles. Everything the write path needs that only one render of
//! one form page can supply.
//!
//! The scrape is anchored-substring rather than a parser. TPT renders these
//! as hidden inputs whose attribute order is not something a capture pins, so
//! each value is found by its `name="…"` anchor, bounded to the tag that
//! carries it, and read out of that tag's `value="…"`. A name that appears
//! twice is a drift signal, not a coin flip: two renders of one field mean
//! the page changed shape under us and the scrape refuses.

use crate::s3::AwsKeyId;

/// The CakePHP `SecurityComponent` triple and the `CsrfProtectionMiddleware`
/// pair, scraped together because they are only valid together: they come
/// from one render of one form URL and the submit replays them verbatim.
///
/// `Debug` redacts. Every member is a credential in the sense that matters —
/// possession of the set plus the cookie jar authorises a write.
#[derive(Clone, PartialEq, Eq)]
pub struct TptFormTokens {
    token_key: String,
    token_fields: String,
    token_unlocked: String,
    csrf_key: String,
    csrf_token: String,
}

impl core::fmt::Debug for TptFormTokens {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TptFormTokens(redacted)")
    }
}

impl TptFormTokens {
    /// The five values in the wire order the submit posts them in: the
    /// `_Token` key and the `_Csrf` pair lead the body, and the `_Token`
    /// `fields`/`unlocked` pair sits mid-body.
    #[must_use]
    pub fn token_key(&self) -> &str {
        &self.token_key
    }

    /// The per-form-URL `SecurityComponent` HMAC. Carried exactly as the
    /// markup rendered it: the trailing `%3A` is part of the value TPT
    /// hashes, so percent-decoding it here would post a string the server
    /// never issued.
    #[must_use]
    pub fn token_fields(&self) -> &str {
        &self.token_fields
    }

    /// The `%7C`-joined unlocked field-name list, likewise verbatim.
    #[must_use]
    pub fn token_unlocked(&self) -> &str {
        &self.token_unlocked
    }

    #[must_use]
    pub fn csrf_key(&self) -> &str {
        &self.csrf_key
    }

    #[must_use]
    pub fn csrf_token(&self) -> &str {
        &self.csrf_token
    }

    /// The field paths the render declares as unlocked, decoded from the
    /// `%7C`-joined list. This is the form's own statement of its shape and
    /// is what the structural probe fingerprints.
    #[must_use]
    pub fn unlocked_field_names(&self) -> Vec<String> {
        percent_decode(&self.token_unlocked)
            .split('|')
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

/// One render of one TPT product form, reduced to what a write needs.
#[derive(Clone, PartialEq, Eq)]
pub struct TptFormPage {
    tokens: TptFormTokens,
    aws_key_id: AwsKeyId,
    thumbs: Vec<ThumbHandle>,
}

impl core::fmt::Debug for TptFormPage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "TptFormPage(redacted, {} thumbs)", self.thumbs.len())
    }
}

impl TptFormPage {
    #[must_use]
    pub const fn tokens(&self) -> &TptFormTokens {
        &self.tokens
    }

    #[must_use]
    pub const fn aws_key_id(&self) -> &AwsKeyId {
        &self.aws_key_id
    }

    /// The existing thumbnail handles an edit render carries, in slot order.
    /// An edit must echo these back or the product loses its thumbnails; a
    /// create render carries none.
    #[must_use]
    pub fn thumbs(&self) -> &[ThumbHandle] {
        &self.thumbs
    }
}

/// An opaque handle to an already-uploaded thumbnail, lifted from the edit
/// render's bootstrap JSON and echoed back verbatim. `Debug` redacts: the
/// handle is a server-side encrypted envelope over an object path.
#[derive(Clone, PartialEq, Eq)]
pub struct ThumbHandle {
    slot: u8,
    key: String,
}

impl core::fmt::Debug for ThumbHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ThumbHandle(slot {}, redacted)", self.slot)
    }
}

impl ThumbHandle {
    #[must_use]
    pub const fn slot(&self) -> u8 {
        self.slot
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
}

/// What a scrape could not establish. Every variant names one anchor, so a
/// drift report says which input moved rather than that the page changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormScrapeError {
    /// The anchor is absent. The commonest cause is the page not being the
    /// form at all — a sign-in interstitial or a challenge answered 200.
    Missing { anchor: &'static str },
    /// The anchor occurs more than once, so no single value is the value.
    Duplicated { anchor: &'static str, count: usize },
    /// The anchor is present but the tag carrying it has no `value`.
    NoValue { anchor: &'static str },
}

impl core::fmt::Display for FormScrapeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missing { anchor } => write!(f, "the form render carries no {anchor}"),
            Self::Duplicated { anchor, count } => {
                write!(f, "the form render carries {count} copies of {anchor}")
            }
            Self::NoValue { anchor } => write!(f, "{anchor} is rendered without a value"),
        }
    }
}

impl core::error::Error for FormScrapeError {}

impl FormScrapeError {
    #[must_use]
    pub const fn anchor(&self) -> &'static str {
        match *self {
            Self::Missing { anchor }
            | Self::Duplicated { anchor, .. }
            | Self::NoValue { anchor } => anchor,
        }
    }
}

const TOKEN_KEY: &str = "data[_Token][key]";
const TOKEN_FIELDS: &str = "data[_Token][fields]";
const TOKEN_UNLOCKED: &str = "data[_Token][unlocked]";
const CSRF_KEY: &str = "data[_Csrf][csrfKey]";
const CSRF_TOKEN: &str = "data[_Csrf][csrfToken]";

/// The anchor the AWS block is found by. The create and edit renders both
/// bootstrap `"aws":{"credentials":{"key":…,"region":…}}` into the page; the
/// key id is a published IAM key id, and the secret never leaves TPT.
const AWS_ANCHOR: &str = "\"credentials\":{\"key\":\"";
const AWS_ANCHOR_NAME: &str = "the aws credentials block";

/// How far past an anchor a bounded scan will look. Generous enough for the
/// longest observed hidden input (the 1185-character unlocked list plus its
/// attributes) and short enough that a missing quote cannot run the scan to
/// the end of a 216 KB page.
const SCAN_WINDOW: usize = 4096;

/// The four manual-thumbnail slots the form declares.
pub const THUMB_SLOTS: u8 = 4;

/// Finds every occurrence of `needle`, refusing at the first sign that the
/// page carries more than one.
fn sole_occurrence(
    html: &str,
    needle: &str,
    anchor: &'static str,
) -> Result<usize, FormScrapeError> {
    let mut found: Option<usize> = None;
    let mut count: usize = 0;
    let mut cursor: usize = 0;
    while let Some(rest) = html.get(cursor..) {
        let Some(offset) = rest.find(needle) else {
            break;
        };
        let at = cursor.saturating_add(offset);
        if found.is_none() {
            found = Some(at);
        }
        count = count.saturating_add(1);
        cursor = at.saturating_add(needle.len().max(1));
    }
    match (found, count) {
        (Some(at), 1) => Ok(at),
        (Some(_), _) => Err(FormScrapeError::Duplicated { anchor, count }),
        (None, _) => Err(FormScrapeError::Missing { anchor }),
    }
}

/// The text between `opener` and the next `closer`, searched inside a bounded
/// window starting at `from`.
fn delimited(
    html: &str,
    from: usize,
    opener: &str,
    closer: char,
    anchor: &'static str,
) -> Result<String, FormScrapeError> {
    let window_end = from.saturating_add(SCAN_WINDOW).min(html.len());
    let window = html
        .get(from..window_end)
        .ok_or(FormScrapeError::NoValue { anchor })?;
    let opens = window
        .find(opener)
        .ok_or(FormScrapeError::NoValue { anchor })?;
    let value_start = opens.saturating_add(opener.len());
    let rest = window
        .get(value_start..)
        .ok_or(FormScrapeError::NoValue { anchor })?;
    let terminates = rest
        .find(closer)
        .ok_or(FormScrapeError::NoValue { anchor })?;
    let raw = rest
        .get(..terminates)
        .ok_or(FormScrapeError::NoValue { anchor })?;
    Ok(raw.to_owned())
}

/// One hidden input's value, found by its `name` and bounded to the tag that
/// declares it. The bound matters: without it a `name` whose own tag has no
/// `value` would silently borrow the next input's.
fn hidden_input(html: &str, name: &'static str) -> Result<String, FormScrapeError> {
    let needle = format!("name=\"{name}\"");
    let at = sole_occurrence(html, &needle, name)?;
    let tag_end = html
        .get(at..)
        .and_then(|rest| rest.find('>'))
        .map_or(html.len(), |offset| {
            at.saturating_add(offset).min(html.len())
        });
    let tag = html
        .get(at..tag_end)
        .ok_or(FormScrapeError::NoValue { anchor: name })?;
    // The leading space is load-bearing: attributes are space-separated, and
    // without it a neighbouring `data-value="…"` would answer for `value`.
    let raw = delimited(tag, 0, " value=\"", '"', name)?;
    Ok(decode_html_entities(&raw))
}

/// The five named entities a CakePHP-escaped attribute can carry. Every
/// observed token value is hex, base64 or percent-encoded and so passes
/// through untouched; the decode exists so a value that does carry one is
/// posted as the browser would post it rather than as the markup spelt it.
fn decode_html_entities(raw: &str) -> String {
    raw.replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Percent-decoding, for reading the unlocked list only. The token values
/// themselves are never decoded — they are posted exactly as rendered.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index: usize = 0;
    while index < bytes.len() {
        let byte = bytes.get(index).copied().unwrap_or(b'%');
        if byte == b'%' {
            let high = bytes.get(index.saturating_add(1)).copied();
            let low = bytes.get(index.saturating_add(2)).copied();
            if let (Some(high), Some(low)) = (high, low) {
                if let (Some(high), Some(low)) = (hex_nibble(high), hex_nibble(low)) {
                    out.push(high.saturating_mul(16).saturating_add(low));
                    index = index.saturating_add(3);
                    continue;
                }
            }
        }
        out.push(byte);
        index = index.saturating_add(1);
    }
    String::from_utf8_lossy(&out).into_owned()
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte.wrapping_sub(b'0')),
        b'a'..=b'f' => Some(byte.wrapping_sub(b'a').wrapping_add(10)),
        b'A'..=b'F' => Some(byte.wrapping_sub(b'A').wrapping_add(10)),
        _ => None,
    }
}

/// JSON string unescaping, for the bootstrap blob only. The handles are
/// base64 and the blob escapes `/` as `\/`, so an exact-substring comparison
/// against the raw page fails until this runs.
fn json_unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut escaped = false;
    for character in raw.chars() {
        if escaped {
            match character {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                other => out.push(other),
            }
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            out.push(character);
        }
    }
    out
}

/// One thumbnail slot's existing handle, if the render carries one. Absent is
/// the normal case — a create render has no uploaded assets, and the product
/// and preview slots carry no `key` member even on an edit — so this reports
/// absence rather than failing.
fn thumb_handle(html: &str, slot: u8) -> Option<ThumbHandle> {
    let anchor = format!("\"name\":\"thumb{slot}\"");
    let at = html.find(&anchor)?;
    let raw = delimited(html, at, "\"key\":\"", '"', "a thumbnail handle").ok()?;
    if raw.is_empty() {
        return None;
    }
    Some(ThumbHandle {
        slot,
        key: json_unescape(&raw),
    })
}

/// Scrapes one form render. Everything the write needs and nothing it does
/// not: no session material is read out of the page, because none is in it.
pub fn scrape_form_page(html: &str) -> Result<TptFormPage, FormScrapeError> {
    let tokens = TptFormTokens {
        token_key: hidden_input(html, TOKEN_KEY)?,
        token_fields: hidden_input(html, TOKEN_FIELDS)?,
        token_unlocked: hidden_input(html, TOKEN_UNLOCKED)?,
        csrf_key: hidden_input(html, CSRF_KEY)?,
        csrf_token: hidden_input(html, CSRF_TOKEN)?,
    };
    let aws_at = sole_occurrence(html, AWS_ANCHOR, AWS_ANCHOR_NAME)?;
    let key_id = delimited(html, aws_at, "\"key\":\"", '"', AWS_ANCHOR_NAME)?;
    let thumbs = (1..=THUMB_SLOTS)
        .filter_map(|slot| thumb_handle(html, slot))
        .collect();
    Ok(TptFormPage {
        tokens,
        aws_key_id: AwsKeyId::new(key_id),
        thumbs,
    })
}

/// Renders of the two form pages, reduced to the parts the scrape reads and
/// with every token replaced by a placeholder of the captured shape. Shared
/// with the write-model and flow tests, which need a token set and can only
/// obtain one by scraping a render.
#[cfg(test)]
pub(crate) mod tests_support {
    /// The 144-character double-submit value, whose shape is 128 hex
    /// characters, an epoch and four digits.
    fn csrf_pair() -> String {
        let hex = "a".repeat(128);
        hex + ".1787893947.7446"
    }

    /// The 43-character per-form-URL hash, percent-encoded colon included.
    fn security_hash() -> String {
        let hex = "1".repeat(40);
        hex + "%3A"
    }

    /// A create render: the five hidden inputs and the AWS bootstrap.
    pub(crate) fn create_render() -> String {
        format!(
            "<html><body><form action=\"/My-Products/New/Digital-Next\" method=\"post\">\
             <input type=\"hidden\" name=\"_method\" value=\"POST\"/>\
             <input type=\"hidden\" name=\"data[_Token][key]\" autocomplete=\"off\" \
             value=\"{key}\"/>\
             <input type=\"hidden\" name=\"data[_Csrf][csrfKey]\" value=\"{csrf}\"/>\
             <input type=\"hidden\" name=\"data[_Csrf][csrfToken]\" value=\"{csrf}\"/>\
             <input type=\"hidden\" name=\"data[_Token][fields]\" value=\"{fields}\"/>\
             <input type=\"hidden\" name=\"data[_Token][unlocked]\" value=\"{unlocked}\"/>\
             </form><script>var cfg = {{\"aws\":{{\"credentials\":\
             {{\"key\":\"AKIAPLACEHOLDER00000\",\"region\":\"us-east-1\"}}}}}};</script>\
             </body></html>",
            key = "0".repeat(40),
            csrf = csrf_pair(),
            fields = security_hash(),
            unlocked = "Item.description%7CItem.name%7CTaxonomyTags%7Cthumbs",
        )
    }

    /// The same render with the four uploaded-thumbnail handles an edit page
    /// carries, slashes JSON-escaped exactly as the bootstrap escapes them.
    pub(crate) fn edit_render() -> String {
        let mut html = create_render();
        html.push_str(
            "<script>var boot = {\"upload_digital\":[\
             {\"name\":\"product\",\"uploaded\":{\"size\":10}},\
             {\"name\":\"thumb1\",\"uploaded\":{\"key\":\"aa\\/bb+cc1=\"}},\
             {\"name\":\"thumb2\",\"uploaded\":{\"key\":\"aa\\/bb+cc2=\"}},\
             {\"name\":\"thumb3\",\"uploaded\":{\"key\":\"aa\\/bb+cc3=\"}},\
             {\"name\":\"thumb4\",\"uploaded\":{\"key\":\"aa\\/bb+cc4=\"}}]};</script>",
        );
        html
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{create_render, edit_render};
    use super::{scrape_form_page, FormScrapeError, ThumbHandle, TptFormTokens};

    #[test]
    fn a_create_render_yields_the_token_triple_and_the_csrf_pair() {
        let page = scrape_form_page(&create_render()).expect("the render carries every anchor");
        assert_eq!(
            page.tokens().token_key().chars().count(),
            40,
            "the SecurityComponent key is the forty hex characters the form rendered"
        );
        assert_eq!(
            page.tokens().csrf_key(),
            page.tokens().csrf_token(),
            "the double submit is one value posted twice, as every capture shows"
        );
        assert_eq!(
            page.aws_key_id().as_str(),
            "AKIAPLACEHOLDER00000",
            "the published access key id comes off the page bootstrap"
        );
        assert!(
            page.thumbs().is_empty(),
            "a create render has no uploaded assets to echo back"
        );
    }

    #[test]
    fn the_security_hash_is_posted_percent_encoded_exactly_as_rendered() {
        let page = scrape_form_page(&create_render()).expect("the render parses");
        assert!(
            page.tokens().token_fields().ends_with("%3A"),
            "decoding the trailing colon would post a string TPT never issued, got {:?}",
            page.tokens().token_fields()
        );
    }

    #[test]
    fn the_unlocked_list_decodes_to_the_field_paths_the_render_declares() {
        let page = scrape_form_page(&create_render()).expect("the render parses");
        assert_eq!(
            page.tokens().unlocked_field_names(),
            vec![
                "Item.description".to_owned(),
                "Item.name".to_owned(),
                "TaxonomyTags".to_owned(),
                "thumbs".to_owned(),
            ],
            "the pipe-joined list is the form's own statement of its shape"
        );
    }

    #[test]
    fn an_edit_render_yields_the_thumbnail_handles_with_their_slashes_restored() {
        let page = scrape_form_page(&edit_render()).expect("the edit render parses");
        assert_eq!(
            page.thumbs().len(),
            4,
            "all four manual slots carry a handle"
        );
        assert_eq!(
            page.thumbs().first().map(ThumbHandle::key),
            Some("aa/bb+cc1="),
            "the bootstrap escapes the slash, and an echoed handle must be the real one"
        );
    }

    #[test]
    fn a_render_missing_an_anchor_names_the_anchor_rather_than_the_page() {
        let html = create_render().replace("name=\"data[_Token][fields]\"", "name=\"other\"");
        assert_eq!(
            scrape_form_page(&html),
            Err(FormScrapeError::Missing {
                anchor: "data[_Token][fields]"
            }),
            "a drift report says which input moved"
        );
    }

    #[test]
    fn a_render_carrying_one_anchor_twice_is_refused_rather_than_guessed() {
        let mut html = create_render();
        html.push_str("<input type=\"hidden\" name=\"data[_Token][key]\" value=\"beef\"/>");
        assert_eq!(
            scrape_form_page(&html),
            Err(FormScrapeError::Duplicated {
                anchor: "data[_Token][key]",
                count: 2
            }),
            "two renders of one field mean the page changed shape, not that either wins"
        );
    }

    #[test]
    fn a_scrape_never_reaches_past_the_tag_that_declares_the_name() {
        let html = create_render().replace(
            "name=\"data[_Token][fields]\" value=",
            "name=\"data[_Token][fields]\" novalue=",
        );
        assert_eq!(
            scrape_form_page(&html),
            Err(FormScrapeError::NoValue {
                anchor: "data[_Token][fields]"
            }),
            "an input with no value must refuse, never borrow the next input's"
        );
    }

    #[test]
    fn the_tokens_never_print_themselves() {
        let page = scrape_form_page(&create_render()).expect("the render parses");
        let printed = format!("{:?} {:?}", page, page.tokens());
        assert!(
            !printed.contains("0000") && !printed.contains("aaaa"),
            "a token in a log line is a credential leak, and Debug printed: {printed}"
        );
        assert_eq!(
            format!("{:?}", page.tokens()),
            "TptFormTokens(redacted)",
            "the redaction is total, not partial"
        );
    }

    #[test]
    fn a_sign_in_page_answered_two_hundred_fails_the_scrape_at_the_first_anchor() {
        let refused = scrape_form_page("<html><body>Please sign-in</body></html>");
        assert!(
            matches!(refused, Err(FormScrapeError::Missing { .. })),
            "a page that is not the form carries no anchor at all, got {refused:?}"
        );
    }

    #[test]
    fn tokens_compare_by_value_so_a_replay_can_be_asserted() {
        let one = scrape_form_page(&create_render()).expect("the render parses");
        let two = scrape_form_page(&create_render()).expect("the render parses");
        let (one, two): (&TptFormTokens, &TptFormTokens) = (one.tokens(), two.tokens());
        assert_eq!(one, two, "one render scraped twice is one token set");
    }
}
