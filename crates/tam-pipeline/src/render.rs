//! Cover and preview generation. Cover generation is a HARD requirement on
//! Tes — it generates neither a cover nor a preview for ZIP uploads — so a
//! failed cover blocks the publish, which `RenderError::CoverRequired`
//! carries distinctly from a best-effort preview failure. Every render is
//! deterministic so an identical payload dedups to one cover blob.
//!
//! Where a picture of the resource exists inside the resource, that picture
//! is the cover. Three places hold one: an image payload is its own cover; a
//! bundle carries the seller's own preview images beside the worksheet; and
//! an OOXML document carries `docProps/thumbnail`, the preview its authoring
//! tool stored. All three are the resource's own bytes, already in hand on
//! the device that fetched them, read under the archive rails in
//! [`crate::archive`] — no marketplace request, no server-side fetch, and no
//! new dependency is involved in finding them.
//!
//! Real first-page PDF rasterisation still needs a native renderer and ships
//! with deploy, and an OOXML document that stored no thumbnail part cannot be
//! rasterised either. Those get an honest generated card, and
//! [`CoverSource`] says so, rather than a fabricated screenshot of content we
//! did not render.

use std::sync::LazyLock;

use image::{DynamicImage, ImageError, ImageFormat, Rgba, RgbaImage};
use tam_types::{ContentHash, FileKind};

use crate::archive::{self, ExtractBudget};

/// The fixed cover dimensions. A single size keeps covers deduplicable and
/// predictable for the upload flow; the marketplace rescales as it needs.
pub const COVER_WIDTH: u32 = 512;
pub const COVER_HEIGHT: u32 = 384;

/// How many entries a preview search inflates before it settles for a card.
///
/// The candidates are ranked before any of them is inflated, so this bounds
/// the work rather than the choice: a bundle of two hundred page images still
/// yields its first page. Each inflation is one entry under
/// [`ExtractBudget::default`] — the same rails the payload itself arrived
/// under — so the peak held here is one entry, not an archive.
const PREVIEW_ATTEMPTS_MAX: usize = 4;

/// How many nested documents inside a bundle are opened for their stored
/// thumbnail, once the bundle's own entries have offered no picture.
const NESTED_ATTEMPTS_MAX: usize = 2;

/// The smallest side a picture must have to be this resource's preview.
/// Below it, a picture is a logo, a bullet or a rule — decoration the
/// document carries rather than a look at the resource.
const PREVIEW_MIN_SIDE: u32 = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedImage {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Where a cover's picture came from.
///
/// Provenance travels with the bytes because the two are not
/// interchangeable: a preview drawn from the resource shows the resource, and
/// a generated card shows only its kind. A caller that cannot tell them apart
/// would report a card as a thumbnail, which is the claim this type exists to
/// make impossible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverSource {
    /// The payload is itself a picture, downscaled into the cover frame.
    Payload,
    /// A picture the resource carries: an image entry in the seller's bundle,
    /// or a document's own stored thumbnail part. The path is archive-relative,
    /// and a nested document's part reads `outer.docx!docProps/thumbnail.jpeg`.
    Embedded { path: String },
    /// No picture exists in these bytes, so the cover is the generated card
    /// for this kind and claims nothing about the content.
    Generated { kind: FileKind },
}

/// A cover, and what it is a picture of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawnCover {
    pub image: RenderedImage,
    pub source: CoverSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// The cover could not be produced, which blocks the publish because Tes
    /// requires one and will not generate it.
    CoverRequired { detail: String },
    /// A best-effort preview failed; the publish proceeds without it.
    PreviewUnavailable { detail: String },
}

impl core::fmt::Display for RenderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CoverRequired { detail } => write!(f, "cover generation failed: {detail}"),
            Self::PreviewUnavailable { detail } => write!(f, "preview unavailable: {detail}"),
        }
    }
}

impl core::error::Error for RenderError {}

fn encode_png(canvas: &RgbaImage) -> Result<Vec<u8>, ImageError> {
    let mut png = Vec::new();
    canvas.write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)?;
    Ok(png)
}

/// A deterministic placeholder card: a solid ground whose colour is derived
/// from the payload kind, with a framed border, at the fixed cover size. It
/// carries no text glyphs because font rendering is a native-closure
/// dependency deferred to deploy; the card is honest about being generated.
fn placeholder_card(kind: FileKind) -> Result<RenderedImage, ImageError> {
    let (r, g, b) = match kind {
        FileKind::Pdf => (0xC0, 0x39, 0x2B),
        FileKind::Pptx => (0xD3, 0x5F, 0x2B),
        FileKind::Docx => (0x2E, 0x5A, 0x88),
        FileKind::Zip => (0x5B, 0x6B, 0x73),
        FileKind::Image => (0x3C, 0x6E, 0x47),
    };
    let mut canvas = RgbaImage::from_pixel(COVER_WIDTH, COVER_HEIGHT, Rgba([r, g, b, 0xFF]));
    let border = Rgba([0xFF, 0xFF, 0xFF, 0xFF]);
    let inset = 16u32;
    for x in inset..COVER_WIDTH - inset {
        canvas.put_pixel(x, inset, border);
        canvas.put_pixel(x, COVER_HEIGHT - inset - 1, border);
    }
    for y in inset..COVER_HEIGHT - inset {
        canvas.put_pixel(inset, y, border);
        canvas.put_pixel(COVER_WIDTH - inset - 1, y, border);
    }
    Ok(RenderedImage {
        png: encode_png(&canvas)?,
        width: COVER_WIDTH,
        height: COVER_HEIGHT,
    })
}

/// Every kind whose card this renderer can draw, which is every kind: the
/// card is the fallback for a payload no picture could be found in.
const CARD_KINDS: [FileKind; 5] = [
    FileKind::Pdf,
    FileKind::Pptx,
    FileKind::Docx,
    FileKind::Zip,
    FileKind::Image,
];

/// The digests the generated cards are stored under.
///
/// Drawn and hashed once here rather than written down as constants, so the
/// classification cannot drift from what [`placeholder_card`] actually
/// produces: a card whose colours changed would be reclassified by the same
/// edit that changed it, where a hardcoded digest list would quietly go on
/// naming pictures nobody draws any more.
static CARD_DIGESTS: LazyLock<Vec<ContentHash>> = LazyLock::new(|| {
    CARD_KINDS
        .iter()
        .filter_map(|kind| placeholder_card(*kind).ok())
        .map(|card| crate::hash::content_hash(&card.png))
        .collect()
});

/// Whether a stored cover is one of this renderer's generated cards.
///
/// The question a repair has to be able to ask. A resource whose thumbnail is
/// a card has no picture of itself, so a later read that found one may replace
/// it; a resource whose thumbnail is anything else has a picture somebody
/// meant it to have — the seller's own upload, or an earlier read's genuine
/// preview — and no automatic write may touch it. Answered from the digest
/// alone, because that is all a `product_file` row carries and it is exact:
/// a card is a pure function of the kind, so its bytes are the same bytes
/// every time and no near-miss can be mistaken for one.
///
/// The card renderer's output is therefore load-bearing beyond what it looks
/// like: changing its colours would leave every cover stored under the old
/// digests unclassifiable, and those resources would keep a card nothing
/// could offer to replace.
#[must_use]
pub fn is_generated_card(hash: ContentHash) -> bool {
    CARD_DIGESTS.contains(&hash)
}

/// Downscales a decoded picture into the fixed cover frame, letterboxed so
/// the aspect ratio is preserved. Deterministic: the same picture yields the
/// same cover, whichever of the two callers decoded it.
fn letterbox(source: &DynamicImage) -> Result<RenderedImage, ImageError> {
    let thumbnail = source.thumbnail(COVER_WIDTH, COVER_HEIGHT).to_rgba8();
    let mut canvas =
        RgbaImage::from_pixel(COVER_WIDTH, COVER_HEIGHT, Rgba([0x11, 0x11, 0x11, 0xFF]));
    // Deliberate flooring: the letterbox offset centres the thumbnail, and a
    // half-pixel bias is invisible and preferable to a float round-trip.
    #[expect(
        clippy::integer_division,
        reason = "centering offset is deliberately floored to an integer pixel"
    )]
    let offset_x = (COVER_WIDTH - thumbnail.width()) / 2;
    #[expect(
        clippy::integer_division,
        reason = "centering offset is deliberately floored to an integer pixel"
    )]
    let offset_y = (COVER_HEIGHT - thumbnail.height()) / 2;
    for (x, y, pixel) in thumbnail.enumerate_pixels() {
        canvas.put_pixel(offset_x + x, offset_y + y, *pixel);
    }
    Ok(RenderedImage {
        png: encode_png(&canvas)?,
        width: COVER_WIDTH,
        height: COVER_HEIGHT,
    })
}

/// Downscales an image payload into the fixed cover frame.
fn cover_from_image(bytes: &[u8]) -> Result<RenderedImage, ImageError> {
    letterbox(&image::load_from_memory(bytes)?)
}

/// The same downscale for a candidate found inside an archive, where a
/// failure is an answer rather than a fault: a picture that will not decode,
/// or is too small to be a preview of anything, is passed over for the next
/// candidate.
fn picture_of(bytes: &[u8]) -> Option<RenderedImage> {
    let source = image::load_from_memory(bytes).ok()?;
    if source.width() < PREVIEW_MIN_SIDE || source.height() < PREVIEW_MIN_SIDE {
        return None;
    }
    letterbox(&source).ok()
}

/// Archive junk no seller authored: the AppleDouble sidecars a macOS zip
/// carries, which hold a resource fork rather than a picture.
fn is_junk(path: &str) -> bool {
    path.starts_with("__MACOSX/")
        || path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with("._"))
}

fn has_suffix(lowered: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|suffix| lowered.ends_with(suffix))
}

/// How good a candidate this path is, or `None` for one that is not a
/// candidate at all.
///
/// Extensions rank candidates and never admit them: what a candidate
/// actually is comes from its leading bytes once it is inflated, so a `.png`
/// entry holding something else is passed over rather than served.
fn rank_of(path: &str) -> Option<u8> {
    if is_junk(path) {
        return None;
    }
    let lowered = path.to_ascii_lowercase();
    // The document's own stored preview, which is the one picture in an OOXML
    // container that was put there to be looked at.
    if has_suffix(
        &lowered,
        &["docprops/thumbnail.jpeg", "docprops/thumbnail.jpg"],
    ) || has_suffix(&lowered, &["docprops/thumbnail.png"])
    {
        return Some(0);
    }
    if !has_suffix(&lowered, &[".png", ".jpg", ".jpeg", ".gif"]) {
        return None;
    }
    let named_as_preview = ["preview", "cover", "thumb"]
        .iter()
        .any(|word| lowered.contains(word));
    Some(if named_as_preview { 1 } else { 2 })
}

/// A picture of the resource, from inside the resource.
///
/// Ranked from the central directory first, so choosing costs the index and
/// only the chosen entries are inflated. Deterministic: the ranking is a
/// total order on `(rank, path)`, so one bundle always yields one cover and
/// two imports of it dedup to one blob.
///
/// One level of nesting, and only after the container's own entries have
/// offered nothing: a bundle of documents is the ordinary Tes shape, and the
/// document's stored thumbnail is the only picture in it.
///
/// `spent` is the whole cover attempt's expansion, not one entry's, and it is
/// the caller's so that the nested search shares it. The per-entry and ratio
/// rails in [`archive::entry`] still apply to every read; this is the
/// additional bound those cannot express — without it, a bundle of a hundred
/// entries could be asked for the cap over and over, once per candidate and
/// again inside every nested document, and the total inflated for one
/// thumbnail would be a multiple of the archive limit.
fn embedded_preview(
    container: &[u8],
    depth: u8,
    budget: ExtractBudget,
    spent: &mut u64,
) -> Option<(String, RenderedImage)> {
    let paths = archive::file_paths(container)?;
    let mut ranked: Vec<(u8, &str)> = paths
        .iter()
        .filter_map(|path| rank_of(path).map(|rank| (rank, path.as_str())))
        .collect();
    ranked.sort_unstable();
    for (_, path) in ranked.iter().take(PREVIEW_ATTEMPTS_MAX) {
        let Some(found) = read_within(container, path, budget, spent) else {
            continue;
        };
        // The signature alone. `probe_kind` would reach inside anything
        // beginning `PK` for an OOXML part, inflating a member of an entry
        // that was itself just inflated — expansion outside every budget
        // above. What a candidate is, is decided here and then by the
        // decoder.
        if !crate::probe::is_image(&found.bytes) {
            continue;
        }
        if let Some(image) = picture_of(&found.bytes) {
            return Some((found.path, image));
        }
    }
    if depth > 0 {
        return None;
    }
    let mut nested: Vec<&str> = paths
        .iter()
        .map(String::as_str)
        .filter(|path| !is_junk(path))
        .filter(|path| {
            let lowered = path.to_ascii_lowercase();
            has_suffix(&lowered, &[".docx", ".pptx"])
        })
        .collect();
    nested.sort_unstable();
    for path in nested.into_iter().take(NESTED_ATTEMPTS_MAX) {
        let Some(found) = read_within(container, path, budget, spent) else {
            continue;
        };
        // No probe: whether these bytes are a document is answered by whether
        // a picture can be found in them, and that search reads the central
        // directory and at most the parts it chooses — under the same running
        // total. Asking `probe_kind` first would inflate a part nobody
        // counted only to learn what the search establishes anyway.
        if let Some((inner, image)) = embedded_preview(&found.bytes, depth + 1, budget, spent) {
            return Some((format!("{path}!{inner}"), image));
        }
    }
    None
}

/// One entry, read against the cover attempt's running total.
///
/// `None` for an entry the archive does not hold, one the rails refused, and
/// one there is no total left for — all three mean the same thing to the
/// caller: try the next candidate, or settle for the card.
///
/// The total is charged inside [`archive::entry`], as the bytes arrive, so a
/// candidate that inflated most of itself and then failed — a corrupt stream,
/// a bad CRC, a cap tripped near the end — has still spent what it inflated.
/// Subtracting a remaining budget here and crediting it only on success would
/// make a bundle of deliberately broken members free to retry, which is the
/// hole this shape closes.
fn read_within(
    container: &[u8],
    path: &str,
    budget: ExtractBudget,
    spent: &mut u64,
) -> Option<archive::ExtractedEntry> {
    if *spent >= budget.total_bytes_max {
        return None;
    }
    archive::entry(container, path, budget, spent).ok()?
}

fn card(kind: FileKind) -> Result<DrawnCover, RenderError> {
    placeholder_card(kind)
        .map(|image| DrawnCover {
            image,
            source: CoverSource::Generated { kind },
        })
        .map_err(|error| RenderError::CoverRequired {
            detail: error.to_string(),
        })
}

/// The cover for a payload, and what it is a picture of.
///
/// An image payload is downscaled. A zip container — a Tes bundle, a `.docx`
/// or a `.pptx` — is searched for the picture it already carries. A PDF, and
/// a container carrying no picture, gets the generated card: this device has
/// no page rasteriser, and inventing one pixel of a page we did not render
/// would be a thumbnail that lies. A failure is `CoverRequired`, which blocks
/// the publish.
///
/// # Errors
///
/// An image payload that will not decode, or an encode that fails.
pub fn cover(kind: FileKind, payload: &[u8]) -> Result<DrawnCover, RenderError> {
    match kind {
        FileKind::Image => cover_from_image(payload)
            .map(|image| DrawnCover {
                image,
                source: CoverSource::Payload,
            })
            .map_err(|error| RenderError::CoverRequired {
                detail: error.to_string(),
            }),
        FileKind::Zip | FileKind::Docx | FileKind::Pptx => {
            // One budget for the whole attempt: every candidate and every
            // nested document draws down the same total.
            let mut spent = 0u64;
            match embedded_preview(payload, 0, ExtractBudget::default(), &mut spent) {
                Some((path, image)) => Ok(DrawnCover {
                    image,
                    source: CoverSource::Embedded { path },
                }),
                None => card(kind),
            }
        }
        FileKind::Pdf => card(kind),
    }
}

/// A best-effort preview: the same rendering, but a failure is non-blocking.
///
/// # Errors
///
/// Whatever [`cover`] refuses, reported as non-blocking.
pub fn preview(kind: FileKind, payload: &[u8]) -> Result<DrawnCover, RenderError> {
    cover(kind, payload).map_err(|error| RenderError::PreviewUnavailable {
        detail: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{cover, is_generated_card, CoverSource, RenderedImage, COVER_HEIGHT, COVER_WIDTH};
    use image::{ImageFormat, Rgba, RgbaImage};
    use std::io::{Cursor, Write};
    use tam_types::FileKind;
    use zip::write::SimpleFileOptions;

    const MAGENTA: [u8; 4] = [0xE0, 0x10, 0xA0, 0xFF];

    fn png_of(width: u32, height: u32, colour: [u8; 4]) -> Vec<u8> {
        let canvas = RgbaImage::from_pixel(width, height, Rgba(colour));
        let mut png = Vec::new();
        canvas
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .expect("encode fixture");
        png
    }

    fn jpeg_of(width: u32, height: u32, colour: [u8; 4]) -> Vec<u8> {
        let canvas = RgbaImage::from_pixel(width, height, Rgba(colour));
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgba8(canvas)
            .to_rgb8()
            .write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
            .expect("encode fixture");
        jpeg
    }

    fn tiny_png() -> Vec<u8> {
        png_of(8, 8, [0x20, 0x80, 0xC0, 0xFF])
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            writer.start_file(*name, options).expect("start entry");
            writer.write_all(bytes).expect("write entry");
        }
        writer.finish().expect("finish archive");
        buffer.into_inner()
    }

    /// A document of the kind `probe_kind` recognises: an OOXML container
    /// naming wordprocessingml, carrying whatever parts the test needs.
    fn docx_with(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let content_types = br#"<?xml version="1.0"?><Types><Override ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
        let mut entries: Vec<(&str, &[u8])> = vec![
            ("[Content_Types].xml", content_types.as_slice()),
            ("word/document.xml", b"<w:document/>"),
        ];
        entries.extend_from_slice(parts);
        zip_with(&entries)
    }

    fn assert_valid_cover(rendered: &RenderedImage) {
        assert_eq!(
            (rendered.width, rendered.height),
            (COVER_WIDTH, COVER_HEIGHT),
            "the cover is the fixed size"
        );
        let decoded = image::load_from_memory(&rendered.png).expect("the cover is a valid PNG");
        assert_eq!(
            (decoded.width(), decoded.height()),
            (COVER_WIDTH, COVER_HEIGHT),
            "the decoded cover has the fixed dimensions"
        );
    }

    /// The colour at the middle of a cover, which for a uniform preview is
    /// the preview's own colour and for a card is the card's ground.
    #[expect(
        clippy::integer_division,
        reason = "the centre pixel is deliberately floored to an integer coordinate"
    )]
    fn centre_colour(rendered: &RenderedImage) -> [u8; 4] {
        let decoded = image::load_from_memory(&rendered.png)
            .expect("the cover is a valid PNG")
            .to_rgba8();
        decoded.get_pixel(COVER_WIDTH / 2, COVER_HEIGHT / 2).0
    }

    fn is_near(found: [u8; 4], wanted: [u8; 4]) -> bool {
        found
            .iter()
            .zip(wanted.iter())
            .all(|(a, b)| a.abs_diff(*b) <= 8)
    }

    #[test]
    fn a_bundles_own_preview_image_becomes_the_cover() {
        // The ordinary Tes shape: a worksheet and the seller's own preview
        // picture of it, in one bundle.
        let bundle = zip_with(&[
            ("worksheet.pdf", b"%PDF-1.7 the worksheet"),
            ("preview-1.png", &png_of(300, 200, MAGENTA)),
        ]);
        let drawn = cover(FileKind::Zip, &bundle).expect("a bundle cover");
        assert_valid_cover(&drawn.image);
        assert_eq!(
            drawn.source,
            CoverSource::Embedded {
                path: "preview-1.png".to_owned()
            },
            "the cover is the bundle's own picture, and says which entry it came from"
        );
        assert!(
            is_near(centre_colour(&drawn.image), MAGENTA),
            "the cover shows the preview's pixels rather than a kind-coloured ground"
        );
        let card = cover(FileKind::Zip, b"not an archive at all").expect("the zip card");
        assert_ne!(
            drawn.image.png, card.image.png,
            "a resource with a preview does not render as the card every zip shares"
        );
    }

    #[test]
    fn a_documents_stored_thumbnail_becomes_its_cover() {
        let document = docx_with(&[("docProps/thumbnail.jpeg", &jpeg_of(240, 180, MAGENTA))]);
        let drawn = cover(FileKind::Docx, &document).expect("a document cover");
        assert_eq!(
            drawn.source,
            CoverSource::Embedded {
                path: "docProps/thumbnail.jpeg".to_owned()
            },
            "the document's own stored preview is the cover"
        );
        assert!(
            is_near(centre_colour(&drawn.image), MAGENTA),
            "the cover shows the stored thumbnail's pixels"
        );
    }

    #[test]
    fn a_nested_documents_thumbnail_is_found_one_level_down() {
        // A bundle of documents and no loose pictures, which is what a Tes
        // resource usually is.
        let bundle = zip_with(&[
            ("readme.txt", b"how to use this pack"),
            (
                "lesson.docx",
                &docx_with(&[("docProps/thumbnail.jpeg", &jpeg_of(240, 180, MAGENTA))]),
            ),
        ]);
        let drawn = cover(FileKind::Zip, &bundle).expect("a nested cover");
        assert_eq!(
            drawn.source,
            CoverSource::Embedded {
                path: "lesson.docx!docProps/thumbnail.jpeg".to_owned()
            },
            "the provenance names the document and the part inside it"
        );
        assert!(
            is_near(centre_colour(&drawn.image), MAGENTA),
            "the cover shows the nested document's thumbnail"
        );
    }

    #[test]
    fn a_resource_carrying_no_picture_gets_the_card_and_says_so() {
        let bundle = zip_with(&[
            ("worksheet.pdf", b"%PDF-1.7 the worksheet"),
            ("answers.pdf", b"%PDF-1.7 the answers"),
        ]);
        let drawn = cover(FileKind::Zip, &bundle).expect("a card");
        assert_valid_cover(&drawn.image);
        assert_eq!(
            drawn.source,
            CoverSource::Generated {
                kind: FileKind::Zip
            },
            "nothing in these bytes is a picture, and the cover claims nothing else"
        );

        // A PDF payload is the same answer for the same reason: no page
        // rasteriser ships on the device, so no cover claims to be a page.
        let pdf = cover(FileKind::Pdf, b"%PDF-1.7 one page").expect("a pdf card");
        assert_eq!(
            pdf.source,
            CoverSource::Generated {
                kind: FileKind::Pdf
            },
            "a page nobody rendered is never reported as a thumbnail of one"
        );
    }

    #[test]
    fn decoration_and_junk_are_not_mistaken_for_a_preview() {
        // An 8x8 bullet, and a macOS resource fork whose name looks like a
        // picture: neither is a look at the resource.
        let bundle = zip_with(&[
            ("worksheet.pdf", b"%PDF-1.7 the worksheet"),
            ("art/bullet.png", &tiny_png()),
            ("__MACOSX/._preview.png", &png_of(300, 200, MAGENTA)),
        ]);
        let drawn = cover(FileKind::Zip, &bundle).expect("a card");
        assert_eq!(
            drawn.source,
            CoverSource::Generated {
                kind: FileKind::Zip
            },
            "an icon is too small to be a preview and an AppleDouble sidecar is not a picture"
        );
    }

    #[test]
    fn what_an_entry_is_comes_from_its_bytes_not_its_name() {
        let bundle = zip_with(&[
            ("preview.png", b"%PDF-1.7 not a picture at all"),
            ("page-1.jpg", &jpeg_of(300, 200, MAGENTA)),
        ]);
        let drawn = cover(FileKind::Zip, &bundle).expect("a bundle cover");
        assert_eq!(
            drawn.source,
            CoverSource::Embedded {
                path: "page-1.jpg".to_owned()
            },
            "the best-named candidate is passed over once its bytes are not a picture"
        );
    }

    #[test]
    fn a_candidate_that_is_itself_an_archive_is_passed_over_rather_than_opened() {
        // The hazard is not the wrong answer, it is the work: asking what
        // these bytes are through the full kind probe would inflate a part
        // inside them — a member of an entry that was itself just inflated,
        // outside every budget the search carries. A picture is recognised by
        // its signature, so a container named like one is simply not a
        // candidate that decodes.
        let inner = docx_with(&[("docProps/thumbnail.jpeg", &jpeg_of(240, 180, MAGENTA))]);
        let bundle = zip_with(&[
            ("preview.png", &inner),
            ("page-1.jpg", &jpeg_of(300, 200, MAGENTA)),
        ]);
        let drawn = cover(FileKind::Zip, &bundle).expect("a bundle cover");
        assert_eq!(
            drawn.source,
            CoverSource::Embedded {
                path: "page-1.jpg".to_owned()
            },
            "the archive wearing a picture's name is skipped, and the real picture wins"
        );
    }

    #[test]
    fn one_budget_covers_the_whole_search_rather_than_each_candidate() {
        // Two candidates, tried in path order: an icon too small to be a
        // preview, then a real page. The budget admits either alone and not
        // both, so a search that reset its budget per candidate would answer
        // with the page and a search that carries one running total cannot
        // reach it.
        let icon = png_of(8, 8, [0x20, 0x80, 0xC0, 0xFF]);
        let page = png_of(300, 200, MAGENTA);
        let bundle = zip_with(&[("1-icon.png", &icon), ("2-page.png", &page)]);
        let budget = super::ExtractBudget {
            total_bytes_max: (icon.len() + page.len() - 1) as u64,
            ratio_max: super::ExtractBudget::default().ratio_max,
        };

        // Under the shared total, the icon's read is charged and the page is
        // past what remains, so the resource settles for its card. Under a
        // budget with room for both, the same bundle yields the page. The
        // pair is the observable difference between one running total and a
        // budget reset per candidate; what exactly a refused read cost is
        // `archive::entry`'s own contract, asserted there against its public
        // counter.
        let mut spent = 0u64;
        assert!(
            super::embedded_preview(&bundle, 0, budget, &mut spent).is_none(),
            "a search that reset its budget per candidate would reach the page; one that \
             carries a running total cannot"
        );

        let mut generous = 0u64;
        let found =
            super::embedded_preview(&bundle, 0, super::ExtractBudget::default(), &mut generous)
                .expect("a preview");
        assert_eq!(
            found.0, "2-page.png",
            "with room for both reads the page is found, so the refusal above is the budget \
             rather than the ranking or the ranking's rejection of the icon"
        );
    }

    #[test]
    fn the_same_resource_always_yields_the_same_cover() {
        let entries: [(&str, Vec<u8>); 3] = [
            ("worksheet.pdf", b"%PDF-1.7 the worksheet".to_vec()),
            ("page-2.png", png_of(300, 200, [0x10, 0x40, 0xE0, 0xFF])),
            ("page-1.png", png_of(300, 200, MAGENTA)),
        ];
        let forwards = zip_with(
            &entries
                .iter()
                .map(|(name, bytes)| (*name, bytes.as_slice()))
                .collect::<Vec<_>>(),
        );
        let backwards = zip_with(
            &entries
                .iter()
                .rev()
                .map(|(name, bytes)| (*name, bytes.as_slice()))
                .collect::<Vec<_>>(),
        );
        let first = cover(FileKind::Zip, &forwards).expect("a cover");
        let second = cover(FileKind::Zip, &backwards).expect("a cover");
        assert_eq!(
            first.source,
            CoverSource::Embedded {
                path: "page-1.png".to_owned()
            },
            "the ranking is a total order on (rank, path), so the first page wins"
        );
        assert_eq!(
            first, second,
            "the choice does not depend on the order the archive was written in, so two \
             imports of one resource dedup to one blob"
        );
        let again = cover(FileKind::Zip, &forwards).expect("a second cover");
        assert_eq!(first, again, "and it is byte-identical on a second render");
    }

    #[test]
    fn every_document_kind_gets_a_valid_deterministic_cover() {
        for kind in [FileKind::Pdf, FileKind::Pptx, FileKind::Docx, FileKind::Zip] {
            let first = cover(kind, b"the payload bytes").expect("a document cover");
            assert_valid_cover(&first.image);
            let again = cover(kind, b"the payload bytes").expect("a second cover");
            assert_eq!(
                first, again,
                "an identical payload yields a byte-identical cover, so dedup works"
            );
        }
    }

    #[test]
    fn an_image_payload_is_downscaled_into_the_cover_frame() {
        let drawn = cover(FileKind::Image, &tiny_png()).expect("an image cover");
        assert_valid_cover(&drawn.image);
        assert_eq!(
            drawn.source,
            CoverSource::Payload,
            "an image payload is its own cover"
        );
    }

    #[test]
    fn a_corrupt_image_payload_blocks_the_cover() {
        let refused = cover(FileKind::Image, b"not actually a png");
        assert!(
            matches!(refused, Err(super::RenderError::CoverRequired { .. })),
            "a cover that cannot be produced blocks the publish"
        );
    }

    /// The digest a Tes bundle's card is stored under, taken from a live
    /// catalogue row: the resource "Statistics Stem & Leaf Plots Editable PPT
    /// L1", whose payload is a 13,002,578-byte zip and whose thumbnail is a
    /// 4,768-byte cover under this hash.
    ///
    /// An independent oracle rather than a value this test computes: it is
    /// what the deployment actually holds, so a card render that drifts from
    /// it stops being able to recognise — and therefore repair — every
    /// thumbnail already stored.
    const LIVE_ZIP_CARD: &str = "bccb9f3b4d2a990c5f5cf4be6be4b3467bffe6c9eb3344c8fee047783557b221";

    fn hex_of(hash: tam_types::ContentHash) -> String {
        use std::fmt::Write as _;
        let mut hex = String::with_capacity(64);
        for byte in hash.0 {
            // infallible on String; the Result is the trait's, not the writer's
            let _unused: core::fmt::Result = write!(hex, "{byte:02x}");
        }
        hex
    }

    #[test]
    fn a_card_is_recognised_by_the_digest_the_catalogue_already_stores() {
        let card = cover(FileKind::Zip, b"not an archive at all").expect("the zip card");
        let digest = crate::hash::content_hash(&card.image.png);
        assert_eq!(
            hex_of(digest),
            LIVE_ZIP_CARD,
            "the card this renderer draws for a zip is byte-identical to the one already \
             stored for every Tes bundle imported so far; if this fails, those covers can no \
             longer be told apart from a picture and become unrepairable"
        );
        assert!(
            is_generated_card(digest),
            "and the renderer classifies its own card, which is what lets a repair replace it"
        );
    }

    #[test]
    fn a_real_preview_is_never_classified_as_a_card() {
        let bundle = zip_with(&[
            ("worksheet.pdf", b"%PDF-1.7 the worksheet"),
            ("preview-1.png", &png_of(300, 200, MAGENTA)),
        ]);
        let drawn = cover(FileKind::Zip, &bundle).expect("a bundle cover");
        assert!(
            !is_generated_card(crate::hash::content_hash(&drawn.image.png)),
            "a picture of the resource is not a card, so no automatic write may replace it"
        );
        // Not `Image`: an image payload is either its own cover or a refusal,
        // so its card is unreachable through this entry point.
        for kind in [FileKind::Pdf, FileKind::Pptx, FileKind::Docx] {
            let card = cover(kind, b"nothing a picture can be found in").expect("a card");
            assert!(
                is_generated_card(crate::hash::content_hash(&card.image.png)),
                "every kind's card is recognisable, not only the zip's"
            );
        }
    }
}
