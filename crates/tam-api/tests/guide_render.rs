//! What a guide body is allowed to become.
//!
//! `tam_api::guides::render` is the one function standing between an author's
//! Markdown and a `{@html}` sink in every seller's browser, and it is the same
//! function for the editor's preview, the operator's detail read and the
//! published page — so this file is the whole of the rendering contract and it
//! needs no database to state it.
//!
//! Every expectation here is written out by hand. Nothing in this file asks
//! the renderer what it produces and then asserts that it produced it: the
//! footnote numbers are literal, because the property is that a reference and
//! its definition carry the same number in reading order, and a test that
//! computed the number from a second renderer would agree with a bug.

use tam_api::guides::render;

/// Footnote definitions written above the paragraph that cites them, in the
/// reverse of the order they are cited.
///
/// The pinned writer numbers a footnote on first encounter of either a
/// reference or a definition, so rendered as written, `b` — the first
/// definition in the source — would take number 1 from `a`, which is cited
/// first. `render` moves definitions after the prose, in reference order, and
/// this is the case that tells the two apart.
const REVERSED: &str = "[^b]: The second note.\n\n\
                        [^a]: The first note.\n\n\
                        Text with [^a] and [^b], and [^a] again.\n\n\
                        [^c]: Nobody cites this.\n";

#[test]
fn a_footnote_and_its_definition_are_numbered_in_reading_order() {
    let html = render(REVERSED);

    // The references, numbered by reading order: 1 for the first cited, 2 for
    // the second, whatever order the definitions were typed in.
    assert!(
        html.contains(r##"<a href="#a">1</a>"##),
        "the first-cited footnote is 1: {html}"
    );
    assert!(
        html.contains(r##"<a href="#b">2</a>"##),
        "and the second-cited is 2: {html}"
    );

    // The definitions carry the same numbers, which is the half that breaks
    // without the reordering.
    assert!(
        html.contains(r#"id="a"><sup class="footnote-definition-label">1</sup>"#),
        "the definition of the first-cited footnote is labelled 1: {html}"
    );
    assert!(
        html.contains(r#"id="b"><sup class="footnote-definition-label">2</sup>"#),
        "and the second's is labelled 2: {html}"
    );

    // Repeated references keep their identity rather than taking a new
    // number: three references, two footnotes.
    assert_eq!(
        html.matches(r##"<a href="#a">1</a>"##).count(),
        2,
        "the twice-cited footnote is cited twice, both times as 1: {html}"
    );

    // Definitions land after the prose, in citation order.
    let prose = html
        .find("Text with")
        .expect("the paragraph is rendered somewhere");
    let first = html
        .find(r#"id="a">"#)
        .expect("the first definition is there");
    let second = html
        .find(r#"id="b">"#)
        .expect("the second definition is there");
    assert!(
        prose < first && first < second,
        "definitions render after the prose and in citation order: {html}"
    );

    // A definition nothing cites is not dropped: it is an author's paragraph,
    // and it renders after the ones that were cited.
    let unreferenced = html
        .find(r#"id="c">"#)
        .expect("the uncited definition is kept");
    assert!(
        second < unreferenced,
        "an uncited definition renders last: {html}"
    );
    assert!(
        html.contains(r#"id="c"><sup class="footnote-definition-label">3</sup>"#),
        "numbered after the cited ones: {html}"
    );
}

#[test]
fn a_footnote_written_below_its_reference_is_numbered_the_same_way() {
    // The ordinary order, which the reordering must leave alone.
    let html = render("Cite [^one].\n\n[^one]: The note.\n");
    assert!(
        html.contains(r##"<a href="#one">1</a>"##),
        "the only reference is 1: {html}"
    );
    assert!(
        html.contains(r#"id="one"><sup class="footnote-definition-label">1</sup>"#),
        "and so is its definition: {html}"
    );
}

#[test]
fn an_unsafe_link_destination_reaches_no_href() {
    let html = render(
        "[a](javascript:alert(1)) [b](JaVaScRiPt:alert(1)) [c](//evil.example/x) \
         [d](data:text/html,x) [e](vbscript:x) [f](file:///etc/passwd)\n",
    );
    assert!(
        !html.contains("href"),
        "not one of these destinations produces an anchor: {html}"
    );
    for word in ["a", "b", "c", "d", "e", "f"] {
        assert!(
            html.contains(word),
            "while the author's own words survive as words: {html}"
        );
    }

    // The evasions that work by making the scheme unreadable to a naive
    // check. A browser strips the control character and the whitespace before
    // parsing, so this module refuses the destination outright.
    for evasion in [
        "[x](java\u{0}script:alert(1))",
        "[x](java\tscript:alert(1))",
        "[x]( javascript:alert(1))",
        "[x](JAVASCRIPT:alert(1))",
    ] {
        let html = render(evasion);
        assert!(
            !html.contains("href"),
            "{evasion} produces no anchor: {html}"
        );
    }

    // An autolink is a link, and is filtered at the same seam.
    let html = render("<javascript:alert(1)>\n");
    assert!(
        !html.contains("href"),
        "an autolink cannot smuggle a scheme past the filter: {html}"
    );
}

#[test]
fn a_safe_link_destination_is_rendered_unchanged() {
    let html = render(
        "[d](https://ok.example/x) [e](mailto:help@ok.example) [f](/guides/x) \
         [g](#part) [i](notes/9:30)\n",
    );
    for expected in [
        r#"<a href="https://ok.example/x">d</a>"#,
        r#"<a href="mailto:help@ok.example">e</a>"#,
        r#"<a href="/guides/x">f</a>"#,
        r##"<a href="#part">g</a>"##,
        // A colon after a slash is a path, not a scheme: this is the relative
        // reference it looks like.
        r#"<a href="notes/9:30">i</a>"#,
    ] {
        assert!(
            html.contains(expected),
            "{expected} is rendered as itself: {html}"
        );
    }
}

#[test]
fn a_direct_https_image_is_rendered_with_its_referrer_suppressed() {
    let html = render("![alt one](https://img.example/a.png)\n");
    assert_eq!(
        html,
        "<p><img src=\"https://img.example/a.png\" alt=\"alt one\" \
         referrerpolicy=\"no-referrer\" loading=\"lazy\" /></p>\n",
        "a direct HTTPS image is permitted, and the address of the guide the \
         seller is reading is not disclosed to the image's host"
    );

    let uploaded = render("![alt two](/v1/guides/images/abc)\n");
    assert_eq!(
        uploaded,
        "<p><img src=\"/v1/guides/images/abc\" alt=\"alt two\" \
         referrerpolicy=\"no-referrer\" loading=\"lazy\" /></p>\n",
        "and so is a picture uploaded through the operator's own route"
    );
}

/// A destination that looks root-relative to a naive reader and external to a
/// browser.
///
/// WHATWG URL parsing treats a backslash in the authority position exactly as
/// a forward slash, so `/\img.example/x` is parsed as `//img.example/x` and
/// fetched from somebody else's host — which for an image means this console
/// disclosing to that host that a seller is reading a guide, and for a link
/// means an off-site navigation that reads as an internal one. A check that
/// admitted anything starting with `/` would admit both.
///
/// Every backslash is refused rather than the two shapes above, because a
/// Markdown destination has no use for one: the platform's own paths and an
/// author's HTTPS URL both spell separators with a forward slash, and a URL
/// carrying a literal backslash would spell it `%5C`.
#[test]
fn a_backslash_cannot_smuggle_an_authority_past_a_leading_slash() {
    for destination in [
        "/\\img.example/x",
        "\\\\img.example/x",
        "/\\\\img.example/x",
    ] {
        let image = render(&format!("![alt]({destination})\n"));
        assert_eq!(
            image, "<p>alt</p>\n",
            "{destination} is not an image source: {image}"
        );

        let link = render(&format!("[word]({destination})\n"));
        assert!(
            !link.contains("href"),
            "{destination} is not a link destination either: {link}"
        );
        assert!(
            link.contains("word"),
            "and the author's word survives as a word: {link}"
        );
    }

    // The forward-slash path this is guarding stays admitted, or the guard is
    // just a ban on pictures.
    let uploaded = render("![alt](/v1/guides/images/abc)\n");
    assert!(
        uploaded.contains(r#"src="/v1/guides/images/abc""#),
        "a genuinely site-relative source is still an image: {uploaded}"
    );

    // `\/` is not an authority trick and must not be refused as one:
    // CommonMark unescapes a backslash-escaped punctuation character inside a
    // destination, so `\/img.example/x` reaches this module as
    // `/img.example/x` — one slash, no authority, the console's own origin.
    // Asserted rather than left to inference, because the difference between
    // an escape the parser resolves and a separator a browser rewrites is
    // exactly what this policy has to get right.
    let escaped = render("![alt](\\/img.example/x)\n");
    assert!(
        escaped.contains(r#"src="/img.example/x""#),
        "an escaped slash is a slash, and the result is same-origin: {escaped}"
    );
}

#[test]
fn an_unsafe_image_destination_reaches_no_src() {
    for body in [
        "![alt](data:image/png;base64,AAA)\n",
        "![alt](//img.example/a.png)\n",
        "![alt](javascript:alert(1))\n",
        "![alt](http://img.example/a.png)\n",
    ] {
        let html = render(body);
        assert!(
            !html.contains("<img"),
            "{body} produces no image element: {html}"
        );
        assert_eq!(
            html, "<p>alt</p>\n",
            "and degrades to the author's own alt text: {html}"
        );
    }

    // An alt text carrying a quotation mark cannot close the attribute of the
    // image that is permitted.
    let html = render("![say \"hello\"](https://img.example/a.png)\n");
    assert_eq!(
        html,
        "<p><img src=\"https://img.example/a.png\" alt=\"say &quot;hello&quot;\" \
         referrerpolicy=\"no-referrer\" loading=\"lazy\" /></p>\n",
        "the alt text is escaped as an attribute value"
    );
}

#[test]
fn raw_html_in_a_body_arrives_as_text() {
    let html = render("<script>alert(1)</script>\n\n<img src=x onerror=alert(1)>\n");
    assert!(
        html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "a block of raw HTML is shown rather than run: {html}"
    );
    assert!(
        html.contains("&lt;img src=x onerror=alert(1)&gt;"),
        "and so is an author's own image tag, which is how it cannot carry an \
         event handler: {html}"
    );
    // The negative oracle is executable markup, not the escaped text. The
    // escaped form necessarily contains the characters of the attack —
    // `onerror=alert(1)` is right there in the shown text, and that is the
    // point of showing it. What must not exist is a tag a browser would act
    // on, so that is what is asserted: no element, therefore no attribute on
    // one, therefore no handler.
    assert!(
        !html.contains("<script") && !html.contains("<img") && !html.contains("<svg"),
        "no tag a browser would act on survives: {html}"
    );

    let inline = render("Text with <b>bold</b> and <span onclick=\"x\">more</span>.\n");
    assert!(
        !inline.contains("<b>") && !inline.contains("<span"),
        "inline raw HTML is escaped by the same mapping: {inline}"
    );
}

#[test]
fn the_markdown_a_guide_is_written_in_still_renders() {
    let html = render(
        "# Heading\n\n| Marketplace | Ships |\n| --- | --- |\n| TPT | yes |\n\n\
         ~~struck~~ and `code`.\n",
    );
    assert!(html.contains("<h1>Heading</h1>"), "headings: {html}");
    assert!(
        html.contains("<table>") && html.contains("<td>TPT</td>"),
        "tables: {html}"
    );
    assert!(html.contains("<del>struck</del>"), "strikethrough: {html}");
    assert!(html.contains("<code>code</code>"), "code spans: {html}");
    assert!(
        !html.contains("id=\"heading\""),
        "and no heading identifier syntax beyond what was asked for: {html}"
    );
}
