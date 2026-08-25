//! Cover and preview generation. Cover generation is a HARD requirement on
//! Tes — it generates neither a cover nor a preview for ZIP uploads — so a
//! failed cover blocks the publish, which `RenderError::CoverRequired`
//! carries distinctly from a best-effort preview failure. Every render is
//! deterministic so an identical payload dedups to one cover blob.
//!
//! Real first-page PDF and PPTX rasterisation needs a native renderer and
//! ships with deploy; for those kinds the cover is an honest generated card,
//! not a fabricated screenshot of content we did not render.

use image::{ImageError, ImageFormat, Rgba, RgbaImage};
use tam_types::FileKind;

/// The fixed cover dimensions. A single size keeps covers deduplicable and
/// predictable for the upload flow; the marketplace rescales as it needs.
pub const COVER_WIDTH: u32 = 512;
pub const COVER_HEIGHT: u32 = 384;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedImage {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
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

/// Downscales an image payload into the fixed cover frame, letterboxed so the
/// aspect ratio is preserved. Deterministic: the same bytes yield the same
/// cover.
fn cover_from_image(bytes: &[u8]) -> Result<RenderedImage, ImageError> {
    let source = image::load_from_memory(bytes)?;
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

/// The cover for a payload. An image payload is downscaled; every other kind
/// gets its deterministic card. A failure is `CoverRequired`, which blocks
/// the publish.
pub fn cover(kind: FileKind, payload: &[u8]) -> Result<RenderedImage, RenderError> {
    let result = match kind {
        FileKind::Image => cover_from_image(payload),
        FileKind::Pdf | FileKind::Pptx | FileKind::Docx | FileKind::Zip => placeholder_card(kind),
    };
    result.map_err(|error| RenderError::CoverRequired {
        detail: error.to_string(),
    })
}

/// A best-effort preview: the same rendering, but a failure is non-blocking.
pub fn preview(kind: FileKind, payload: &[u8]) -> Result<RenderedImage, RenderError> {
    cover(kind, payload).map_err(|error| RenderError::PreviewUnavailable {
        detail: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{cover, RenderedImage, COVER_HEIGHT, COVER_WIDTH};
    use image::{ImageFormat, Rgba, RgbaImage};
    use tam_types::FileKind;

    fn tiny_png() -> Vec<u8> {
        let canvas = RgbaImage::from_pixel(8, 8, Rgba([0x20, 0x80, 0xC0, 0xFF]));
        let mut png = Vec::new();
        canvas
            .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
            .expect("encode fixture");
        png
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

    #[test]
    fn every_document_kind_gets_a_valid_deterministic_cover() {
        for kind in [FileKind::Pdf, FileKind::Pptx, FileKind::Docx, FileKind::Zip] {
            let first = cover(kind, b"the payload bytes").expect("a document cover");
            assert_valid_cover(&first);
            let again = cover(kind, b"the payload bytes").expect("a second cover");
            assert_eq!(
                first, again,
                "an identical payload yields a byte-identical cover, so dedup works"
            );
        }
    }

    #[test]
    fn an_image_payload_is_downscaled_into_the_cover_frame() {
        let cover = cover(FileKind::Image, &tiny_png()).expect("an image cover");
        assert_valid_cover(&cover);
    }

    #[test]
    fn a_corrupt_image_payload_blocks_the_cover() {
        let refused = cover(FileKind::Image, b"not actually a png");
        assert!(
            matches!(refused, Err(super::RenderError::CoverRequired { .. })),
            "a cover that cannot be produced blocks the publish"
        );
    }
}
