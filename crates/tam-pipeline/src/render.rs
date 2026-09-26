//! Cover and preview generation. Cover generation is a HARD requirement on
//! Tes — it generates neither a cover nor a preview for ZIP uploads — so a
//! failed cover blocks the publish, which `RenderError::CoverRequired`
//! carries distinctly from a best-effort preview failure. Every render is
//! deterministic so an identical payload dedups to one cover blob.
//!
//! Where a picture of the resource exists, that picture is the cover. An
//! image payload is its own cover; a PDF's first page is rasterised; a bundle
//! carries the seller's own preview images, or a PDF whose first page is
//! rasterised; and an OOXML document carries `docProps/thumbnail`, the
//! preview its authoring tool stored. All of them are the resource's own
//! bytes, already in hand on the device that fetched them, read under the
//! archive rails in [`crate::archive`] — no marketplace request and no
//! server-side fetch is involved in finding them.
//!
//! Pages are rasterised by `hayro`, which is pure Rust, so every client
//! target still builds without a C toolchain. A PDF is untrusted input, so a
//! page render runs on its own thread against [`PAGE_RENDER_TIMEOUT`] and is
//! drawn straight into the cover frame, which bounds the pixmap whatever
//! page size the file declares.
//!
//! A document nothing can be drawn from — an encrypted or unreadable PDF, an
//! OOXML document that stored no thumbnail — gets an honest generated card,
//! and [`CoverSource`] says so, rather than a fabricated screenshot of
//! content we did not render.

use std::sync::{mpsc, Arc, LazyLock};
use std::time::Duration;

use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::FilterType;
use image::{
    DynamicImage, ExtendedColorType, ImageEncoder, ImageError, ImageFormat, Rgb, RgbImage, Rgba,
    RgbaImage,
};
use tam_types::{ContentHash, FileKind};

use crate::archive::{self, ExtractBudget};

/// The cover frame. 4:3 because Tes stores its covers at 4:3; twice Tes's
/// 800x600 so the marketplace always scales down, never up; and wide enough
/// that TPT's 1000px square (see [`square_cover`]) is a downscale too.
pub const COVER_WIDTH: u32 = 1600;
pub const COVER_HEIGHT: u32 = 1200;

/// The largest cover this renderer produces. It has to fit under two limits:
/// TPT refuses a thumbnail of 4 MB or more, and the device import carries
/// covers under `tam_engine_driver::import::COVER_BYTES_MAX`, which is this
/// same 2 MiB. A cover whose PNG would be larger is redrawn at the next
/// frame down in [`SMALLER_FRAMES`].
pub const COVER_BYTES_MAX: usize = 2 * 1024 * 1024;

/// The frames a cover steps down through when its PNG is over
/// [`COVER_BYTES_MAX`], all 4:3 and none below Tes's own 800x600.
const SMALLER_FRAMES: [(u32, u32); 3] = [(1280, 960), (1024, 768), (800, 600)];

/// The side of the square TPT displays a thumbnail in. TPT asks for at least
/// 750px and centre-crops anything rectangular; [`square_cover`] pads to this
/// square instead so nothing of the cover is cut away.
pub const TPT_SQUARE_SIDE: u32 = 1000;

/// How long one page render may take before the cover settles for the card.
/// A hostile PDF can describe unbounded work; the render thread is abandoned
/// at this deadline rather than joined, so an import never hangs on it.
pub const PAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(10);

/// Below this short side, a stored document thumbnail is a small picture:
/// shown at its native size rather than stretched, and passed over for a
/// bundle's PDF page when one can be rasterised.
const NATIVE_SHORT_SIDE_MIN: u32 = 600;

/// The light ground a picture that does not fill the frame sits on.
const GROUND: Rgb<u8> = Rgb([0xF4, 0xF4, 0xF5]);

/// The ground [`square_cover`] pads onto.
const SQUARE_GROUND: Rgb<u8> = Rgb([0xFF, 0xFF, 0xFF]);

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

/// How many PDFs inside a bundle are offered to the page rasteriser.
const BUNDLE_PDF_ATTEMPTS_MAX: usize = 2;

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
    /// The payload is itself a picture, fitted into the cover frame.
    Payload,
    /// A page of a PDF payload, rasterised into the cover frame. Zero-based.
    Page { index: u32 },
    /// A picture the resource carries: an image entry in the seller's bundle,
    /// or a document's own stored thumbnail part. The path is archive-relative,
    /// and a nested document's part reads `outer.docx!docProps/thumbnail.jpeg`.
    Embedded { path: String },
    /// A page of a PDF inside the seller's bundle, rasterised into the cover
    /// frame. The path is archive-relative; the index is zero-based.
    EmbeddedPage { path: String, index: u32 },
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

/// A cover's PNG. Opaque RGB, because every frame is drawn on a solid ground
/// and an alpha channel would be a third more bytes saying nothing.
fn encode_cover(canvas: &RgbImage) -> Result<Vec<u8>, ImageError> {
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Default, PngFilter::Adaptive)
        .write_image(
            canvas.as_raw(),
            canvas.width(),
            canvas.height(),
            ExtendedColorType::Rgb8,
        )?;
    Ok(png)
}

/// The finished cover: the frame encoded, stepped down through
/// [`SMALLER_FRAMES`] while the PNG is over [`COVER_BYTES_MAX`]. A drawn page
/// is nowhere near the cap; a noisy photograph at full frame can be.
fn finish(canvas: &RgbImage) -> Result<RenderedImage, ImageError> {
    let mut png = encode_cover(canvas)?;
    let (mut width, mut height) = canvas.dimensions();
    for (smaller_width, smaller_height) in SMALLER_FRAMES {
        if png.len() <= COVER_BYTES_MAX {
            break;
        }
        let smaller =
            image::imageops::resize(canvas, smaller_width, smaller_height, FilterType::Lanczos3);
        png = encode_cover(&smaller)?;
        (width, height) = (smaller_width, smaller_height);
    }
    Ok(RenderedImage { png, width, height })
}

/// One pixel of a picture, composited over an opaque ground.
#[expect(
    clippy::integer_division,
    reason = "a rounded 8-bit blend: the division by 255 is the fixed-point rescale"
)]
fn over(pixel: Rgba<u8>, ground: Rgb<u8>) -> Rgb<u8> {
    let alpha = u16::from(pixel.0[3]);
    let blend = |source: u8, under: u8| {
        let mixed = (u16::from(source) * alpha + u16::from(under) * (255 - alpha) + 127) / 255;
        u8::try_from(mixed).unwrap_or(u8::MAX)
    };
    Rgb([
        blend(pixel.0[0], ground.0[0]),
        blend(pixel.0[1], ground.0[1]),
        blend(pixel.0[2], ground.0[2]),
    ])
}

/// A picture centred on a `width` by `height` canvas of `ground`. The
/// picture must already fit.
fn centred(picture: &RgbaImage, width: u32, height: u32, ground: Rgb<u8>) -> RgbImage {
    let mut canvas = RgbImage::from_pixel(width, height, ground);
    // Deliberate flooring: the offset centres the picture, and a half-pixel
    // bias is invisible and preferable to a float round-trip.
    #[expect(
        clippy::integer_division,
        reason = "centering offset is deliberately floored to an integer pixel"
    )]
    let offset_x = width.saturating_sub(picture.width()) / 2;
    #[expect(
        clippy::integer_division,
        reason = "centering offset is deliberately floored to an integer pixel"
    )]
    let offset_y = height.saturating_sub(picture.height()) / 2;
    for (x, y, pixel) in picture.enumerate_pixels() {
        if let Some(slot) = canvas.get_pixel_mut_checked(offset_x + x, offset_y + y) {
            *slot = over(*pixel, ground);
        }
    }
    canvas
}

/// A decoded picture fitted into the cover frame on the light ground.
///
/// Larger than the frame, it is downscaled with a Lanczos filter. Smaller, it
/// is placed at its native size: stretching a 256px stored thumbnail to 1600
/// shows the seller a blur, where a sharp small picture on a light ground is
/// the honest look at what the resource carries. Deterministic: the same
/// picture yields the same cover, whichever caller decoded it.
fn framed(source: &DynamicImage) -> RgbImage {
    let picture = if source.width() <= COVER_WIDTH && source.height() <= COVER_HEIGHT {
        source.to_rgba8()
    } else {
        source
            .resize(COVER_WIDTH, COVER_HEIGHT, FilterType::Lanczos3)
            .to_rgba8()
    };
    centred(&picture, COVER_WIDTH, COVER_HEIGHT, GROUND)
}

/// Fits an image payload into the cover frame.
fn cover_from_image(bytes: &[u8]) -> Result<RenderedImage, ImageError> {
    finish(&framed(&image::load_from_memory(bytes)?))
}

/// A candidate found inside an archive, decoded, where a failure is an answer
/// rather than a fault: a picture that will not decode, or is too small to be
/// a preview of anything, is passed over for the next candidate.
fn picture_of(bytes: &[u8]) -> Option<DynamicImage> {
    let source = image::load_from_memory(bytes).ok()?;
    if source.width() < PREVIEW_MIN_SIDE || source.height() < PREVIEW_MIN_SIDE {
        return None;
    }
    Some(source)
}

/// The pixel size and scale that fit a page of `width` by `height` points
/// into the cover frame, or `None` for a page with no drawable extent.
///
/// The scale is chosen from the frame, never from the page, so a page that
/// declares itself a kilometre wide still renders into at most 1600x1200
/// pixels: the pixmap is bounded by this function, not by the file.
#[expect(
    clippy::cast_precision_loss,
    reason = "1600 and 1200 are exactly representable in f32"
)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "both sides are clamped to 1..=frame before the cast, so they fit a u16"
)]
fn page_fit(width: f32, height: f32) -> Option<(u16, u16, f32)> {
    if !(width.is_finite() && height.is_finite()) || width < 1.0 || height < 1.0 {
        return None;
    }
    let (frame_width, frame_height) = (COVER_WIDTH as f32, COVER_HEIGHT as f32);
    let scale = (frame_width / width).min(frame_height / height);
    let pixel_width = (width * scale).floor().clamp(1.0, frame_width);
    let pixel_height = (height * scale).floor().clamp(1.0, frame_height);
    Some((pixel_width as u16, pixel_height as u16, scale))
}

/// The first page of a PDF, drawn at the cover frame's scale on white.
///
/// `None` for anything `hayro` cannot open — an encrypted file, a damaged
/// one, a document with no pages — which the caller answers with the card.
fn first_page(pdf: Vec<u8>) -> Option<RgbImage> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::vello_cpu::color::palette::css::WHITE;

    let document = Pdf::new(Arc::new(pdf)).ok()?;
    let page = document.pages().first()?;
    let (width, height) = page.render_dimensions();
    let (pixel_width, pixel_height, scale) = page_fit(width, height)?;
    let settings = hayro::RenderSettings {
        x_scale: scale,
        y_scale: scale,
        width: Some(pixel_width),
        height: Some(pixel_height),
        bg_color: WHITE,
    };
    let pixmap = hayro::render(
        page,
        &hayro::RenderCache::new(),
        &InterpreterSettings::default(),
        &settings,
    );
    let (width, height) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
    let raw: Vec<u8> = pixmap
        .take_unpremultiplied()
        .into_iter()
        .flat_map(|pixel| [pixel.r, pixel.g, pixel.b])
        .collect();
    RgbImage::from_raw(width, height, raw)
}

/// [`first_page`] on its own thread, abandoned at [`PAGE_RENDER_TIMEOUT`].
///
/// The rasteriser is synchronous and has no cancellation, so a deadline is
/// the only bound on time a caller can hold it to. A render that overruns is
/// left to finish on its detached thread and its answer is dropped; one that
/// panics drops its sender, which reads here as no page. Either way the
/// cover falls back to the card and the import moves on.
fn rasterise_first_page(pdf: Vec<u8>) -> Option<RgbImage> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("tam-cover-page".to_owned())
        .spawn(move || {
            // The receiver has gone only when the deadline passed; the answer
            // has nobody left to read it.
            drop(sender.send(first_page(pdf)));
        })
        .ok()?;
    receiver.recv_timeout(PAGE_RENDER_TIMEOUT).ok().flatten()
}

/// The kind accents on the generated card's page glyph: muted, and never
/// red, so a card reads as "no picture yet" rather than as an error.
fn card_accent(kind: FileKind) -> Rgb<u8> {
    match kind {
        FileKind::Pdf => Rgb([0x71, 0x71, 0x7A]),
        FileKind::Pptx => Rgb([0xA8, 0x8B, 0x62]),
        FileKind::Docx => Rgb([0x5B, 0x7D, 0xB1]),
        FileKind::Zip => Rgb([0x5E, 0x8C, 0x87]),
        FileKind::Image => Rgb([0x6B, 0x8F, 0x71]),
    }
}

/// Fills the half-open rectangle `[x0, x1) x [y0, y1)`.
fn fill(canvas: &mut RgbImage, (x0, y0): (u32, u32), (x1, y1): (u32, u32), colour: Rgb<u8>) {
    for y in y0..y1 {
        for x in x0..x1 {
            if let Some(slot) = canvas.get_pixel_mut_checked(x, y) {
                *slot = colour;
            }
        }
    }
}

/// The card's page glyph, placed by its top-left corner and sized 3:4.
const GLYPH_LEFT: u32 = 530;
const GLYPH_TOP: u32 = 240;
const GLYPH_WIDTH: u32 = 540;
const GLYPH_HEIGHT: u32 = 720;
const GLYPH_FOLD: u32 = 120;
const _: () = assert!(
    GLYPH_LEFT * 2 + GLYPH_WIDTH == COVER_WIDTH && GLYPH_TOP * 2 + GLYPH_HEIGHT == COVER_HEIGHT,
    "the page glyph is centred in the cover frame"
);

/// A deterministic placeholder card: a neutral light-grey ground with a page
/// glyph drawn from rectangles — a sheet, its folded corner, a kind-tinted
/// heading and text lines — at the cover frame. No text, so no font: the card
/// is honest about being generated and says only that a document is here.
fn placeholder_card(kind: FileKind) -> Result<RenderedImage, ImageError> {
    let ground = Rgb([0xEC, 0xEC, 0xEE]);
    let shadow = Rgb([0xD9, 0xD9, 0xDE]);
    let edge = Rgb([0xB4, 0xB4, 0xBB]);
    let sheet = Rgb([0xFF, 0xFF, 0xFF]);
    let fold = Rgb([0xE2, 0xE2, 0xE6]);
    let line = Rgb([0xD4, 0xD4, 0xD8]);
    let (left, top) = (GLYPH_LEFT, GLYPH_TOP);
    let (right, bottom) = (GLYPH_LEFT + GLYPH_WIDTH, GLYPH_TOP + GLYPH_HEIGHT);
    let mut canvas = RgbImage::from_pixel(COVER_WIDTH, COVER_HEIGHT, ground);
    // The shadow falls below and to the right, and starts under the fold so
    // the cut corner stays clean.
    fill(
        &mut canvas,
        (left + 12, bottom),
        (right + 12, bottom + 12),
        shadow,
    );
    fill(
        &mut canvas,
        (right, top + GLYPH_FOLD),
        (right + 12, bottom),
        shadow,
    );
    fill(&mut canvas, (left, top), (right, bottom), edge);
    fill(
        &mut canvas,
        (left + 6, top + 6),
        (right - 6, bottom - 6),
        sheet,
    );
    // The folded corner: above the diagonal is cut back to the ground, below
    // it is the turned-over flap.
    for dy in 0..GLYPH_FOLD {
        for dx in 0..GLYPH_FOLD {
            let colour = if dx > dy {
                ground
            } else if dx + 6 > dy || dx >= GLYPH_FOLD - 6 {
                edge
            } else {
                fold
            };
            canvas.put_pixel(right - GLYPH_FOLD + dx, top + dy, colour);
        }
    }
    fill(
        &mut canvas,
        (left + 60, top + 150),
        (left + 300, top + 186),
        card_accent(kind),
    );
    for (row, width) in [420u32, 400, 420, 380, 420, 260].into_iter().enumerate() {
        let y = top + 250 + u32::try_from(row).unwrap_or(0) * 64;
        fill(
            &mut canvas,
            (left + 60, y),
            (left + 60 + width, y + 20),
            line,
        );
    }
    finish(&canvas)
}

/// The cards the renderer drew before 2026-09-26: a 512x384 kind-coloured
/// ground (the PDF's red) with a white frame, encoded with the default PNG
/// settings. Nothing draws them as covers any more; they are kept, byte for
/// byte, because every resource imported before then still stores one and
/// [`is_generated_card`] must go on recognising them for a repair to replace
/// them.
fn legacy_card(kind: FileKind) -> Result<Vec<u8>, ImageError> {
    const WIDTH: u32 = 512;
    const HEIGHT: u32 = 384;
    let (r, g, b) = match kind {
        FileKind::Pdf => (0xC0, 0x39, 0x2B),
        FileKind::Pptx => (0xD3, 0x5F, 0x2B),
        FileKind::Docx => (0x2E, 0x5A, 0x88),
        FileKind::Zip => (0x5B, 0x6B, 0x73),
        FileKind::Image => (0x3C, 0x6E, 0x47),
    };
    let mut canvas = RgbaImage::from_pixel(WIDTH, HEIGHT, Rgba([r, g, b, 0xFF]));
    let border = Rgba([0xFF, 0xFF, 0xFF, 0xFF]);
    let inset = 16u32;
    for x in inset..WIDTH - inset {
        canvas.put_pixel(x, inset, border);
        canvas.put_pixel(x, HEIGHT - inset - 1, border);
    }
    for y in inset..HEIGHT - inset {
        canvas.put_pixel(inset, y, border);
        canvas.put_pixel(WIDTH - inset - 1, y, border);
    }
    let mut png = Vec::new();
    canvas.write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)?;
    Ok(png)
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

/// The digests the generated cards are stored under: today's cards and the
/// legacy ones every earlier import stored.
///
/// Drawn and hashed once here rather than written down as constants, so the
/// classification cannot drift from what [`placeholder_card`] and
/// [`legacy_card`] actually produce: a card whose colours changed would be
/// reclassified by the same edit that changed it, where a hardcoded digest
/// list would quietly go on naming pictures nobody draws any more.
static CARD_DIGESTS: LazyLock<Vec<ContentHash>> = LazyLock::new(|| {
    let current = CARD_KINDS
        .iter()
        .filter_map(|kind| placeholder_card(*kind).ok())
        .map(|card| card.png);
    let legacy = CARD_KINDS.iter().filter_map(|kind| legacy_card(*kind).ok());
    current
        .chain(legacy)
        .map(|png| crate::hash::content_hash(&png))
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
/// The card renderers' output is therefore load-bearing beyond what it looks
/// like: changing a card's drawing leaves every cover stored under the old
/// digests unclassifiable unless the old drawing is kept, which is why
/// [`legacy_card`] still exists.
#[must_use]
pub fn is_generated_card(hash: ContentHash) -> bool {
    CARD_DIGESTS.contains(&hash)
}

/// The square TPT shows a thumbnail in, drawn from a cover.
///
/// TPT centre-crops a rectangular thumbnail to a square, which would cut the
/// sides off a 4:3 cover. This pads instead: the cover is fitted to
/// [`TPT_SQUARE_SIDE`] and centred on a white square, so a 1600x1200 cover
/// becomes a 1000x750 band with white above and below, and nothing is lost.
///
/// # Errors
///
/// A cover that will not decode, or an encode that fails.
pub fn square_cover(cover_png: &[u8]) -> Result<RenderedImage, RenderError> {
    let refused = |error: ImageError| RenderError::CoverRequired {
        detail: error.to_string(),
    };
    let source = image::load_from_memory(cover_png).map_err(refused)?;
    let fitted = source
        .resize(TPT_SQUARE_SIDE, TPT_SQUARE_SIDE, FilterType::Lanczos3)
        .to_rgba8();
    let canvas = centred(&fitted, TPT_SQUARE_SIDE, TPT_SQUARE_SIDE, SQUARE_GROUND);
    let png = encode_cover(&canvas).map_err(refused)?;
    Ok(RenderedImage {
        png,
        width: TPT_SQUARE_SIDE,
        height: TPT_SQUARE_SIDE,
    })
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
) -> Option<(String, DynamicImage)> {
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
        if let Some(picture) = picture_of(&found.bytes) {
            return Some((found.path, picture));
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
        if let Some((inner, picture)) = embedded_preview(&found.bytes, depth + 1, budget, spent) {
            return Some((format!("{path}!{inner}"), picture));
        }
    }
    None
}

/// The first page of the first PDF in a bundle that rasterises, under the
/// cover attempt's running total. Candidates in path order, so one bundle
/// always yields one page.
fn bundle_page(
    container: &[u8],
    budget: ExtractBudget,
    spent: &mut u64,
) -> Option<(String, RgbImage)> {
    let paths = archive::file_paths(container)?;
    let mut pdfs: Vec<&str> = paths
        .iter()
        .map(String::as_str)
        .filter(|path| !is_junk(path) && has_suffix(&path.to_ascii_lowercase(), &[".pdf"]))
        .collect();
    pdfs.sort_unstable();
    for path in pdfs.into_iter().take(BUNDLE_PDF_ATTEMPTS_MAX) {
        let Some(found) = read_within(container, path, budget, spent) else {
            continue;
        };
        if !found.bytes.starts_with(b"%PDF") {
            continue;
        }
        if let Some(page) = rasterise_first_page(found.bytes) {
            return Some((found.path, page));
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

/// A drawn frame as the cover, with where it came from.
fn drawn(canvas: &RgbImage, source: CoverSource) -> Result<DrawnCover, RenderError> {
    finish(canvas)
        .map(|image| DrawnCover { image, source })
        .map_err(|error| RenderError::CoverRequired {
            detail: error.to_string(),
        })
}

/// The cover for a container: a bundle, a `.docx` or a `.pptx`.
///
/// The seller's own picture wins when it is sharp enough to fill a good part
/// of the frame. A bundle whose only picture is small, or which has none, is
/// offered to the page rasteriser through its first PDF, because a drawn page
/// at full frame is a better look at a worksheet than a thumbnail-sized
/// preview. What remains is shown at native size, and then the card.
fn container_cover(kind: FileKind, payload: &[u8]) -> Result<DrawnCover, RenderError> {
    // One budget for the whole attempt: every candidate, every nested
    // document and every bundled PDF draws down the same total.
    let budget = ExtractBudget::default();
    let mut spent = 0u64;
    let found = embedded_preview(payload, 0, budget, &mut spent);
    let sharp = found
        .as_ref()
        .is_some_and(|(_, picture)| picture.width().min(picture.height()) >= NATIVE_SHORT_SIDE_MIN);
    if !sharp && kind == FileKind::Zip {
        if let Some((path, page)) = bundle_page(payload, budget, &mut spent) {
            return drawn(
                &page_frame(page),
                CoverSource::EmbeddedPage { path, index: 0 },
            );
        }
    }
    match found {
        Some((path, picture)) => drawn(&framed(&picture), CoverSource::Embedded { path }),
        None => card(kind),
    }
}

/// A rasterised page on the cover frame: it already fits exactly on one axis,
/// so this only centres it on the light ground.
fn page_frame(page: RgbImage) -> RgbImage {
    framed(&DynamicImage::ImageRgb8(page))
}

/// The cover for a payload, and what it is a picture of.
///
/// An image payload is fitted into the frame. A PDF's first page is
/// rasterised into it. A zip container — a Tes bundle, a `.docx` or a
/// `.pptx` — is searched for the picture it already carries, and a bundle's
/// PDF is rasterised when that picture is missing or small. A payload nothing
/// can be drawn from gets the generated card, which [`CoverSource::Generated`]
/// names as such. A failure is `CoverRequired`, which blocks the publish.
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
        FileKind::Zip | FileKind::Docx | FileKind::Pptx => container_cover(kind, payload),
        FileKind::Pdf => match rasterise_first_page(payload.to_vec()) {
            Some(page) => drawn(&page_frame(page), CoverSource::Page { index: 0 }),
            None => card(kind),
        },
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
    use super::{
        cover, is_generated_card, square_cover, CoverSource, RenderedImage, COVER_BYTES_MAX,
        COVER_HEIGHT, COVER_WIDTH, TPT_SQUARE_SIDE,
    };
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

        // A PDF the rasteriser cannot open is the same answer: no cover
        // claims to be a page nobody rendered.
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
    fn a_legacy_card_is_recognised_by_the_digest_the_catalogue_already_stores() {
        let legacy = super::legacy_card(FileKind::Zip).expect("the legacy zip card");
        let digest = crate::hash::content_hash(&legacy);
        assert_eq!(
            hex_of(digest),
            LIVE_ZIP_CARD,
            "the legacy zip card is byte-identical to the one already stored for every Tes \
             bundle imported before 2026-09-26; if this fails, those covers can no longer be \
             told apart from a picture and become unrepairable"
        );
        assert!(
            is_generated_card(digest),
            "a legacy card is still a card, which is what lets a repair replace it"
        );
        let current = cover(FileKind::Zip, b"not an archive at all").expect("the zip card");
        assert!(
            is_generated_card(crate::hash::content_hash(&current.image.png)),
            "and so is the card drawn today"
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

    /// A one-page PDF, written out with a correct cross-reference table so
    /// the rasteriser reads it the way it reads a real file rather than
    /// through its repair path. The page uses Helvetica, one of the standard
    /// fonts a PDF need not embed, so the text also proves a substitute font
    /// is reached.
    fn pdf_with(media_box: &str, content: &str) -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [{media_box}] /Contents 4 0 R \
                 /Resources << /Font << /F1 5 0 R >> >> >>"
            ),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len() + 1
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    /// A US Letter worksheet: a blue heading band and a line of black text.
    fn worksheet_pdf() -> Vec<u8> {
        pdf_with(
            "0 0 612 792",
            "0.2 0.4 0.8 rg 72 600 468 120 re f\n\
             BT /F1 36 Tf 0 0 0 rg 72 500 Td (Fractions worksheet) Tj ET",
        )
    }

    fn decoded(rendered: &RenderedImage) -> RgbaImage {
        image::load_from_memory(&rendered.png)
            .expect("the cover is a valid PNG")
            .to_rgba8()
    }

    const GROUND: [u8; 4] = [0xF4, 0xF4, 0xF5, 0xFF];

    #[test]
    fn a_pdfs_first_page_becomes_its_cover() {
        let pdf = worksheet_pdf();
        let drawn = cover(FileKind::Pdf, &pdf).expect("a page cover");
        assert_eq!(
            drawn.source,
            CoverSource::Page { index: 0 },
            "the cover says it is the first page"
        );
        assert_valid_cover(&drawn.image);
        assert!(
            !is_generated_card(crate::hash::content_hash(&drawn.image.png)),
            "a drawn page is a picture of the resource, not a card"
        );
        let pixels = decoded(&drawn.image);
        // Letter at 1200px tall is 927px wide, centred: the heading band
        // spans roughly y 109..291 and x 445..1154.
        assert!(
            is_near(pixels.get_pixel(800, 200).0, [0x33, 0x66, 0xCC, 0xFF]),
            "the page's own heading band is drawn where the page puts it, found {:?}",
            pixels.get_pixel(800, 200).0
        );
        assert!(
            is_near(pixels.get_pixel(100, 600).0, GROUND),
            "the page sits on the light ground, not a dark one"
        );
        assert!(
            is_near(pixels.get_pixel(800, 1000).0, [0xFF, 0xFF, 0xFF, 0xFF]),
            "the page itself is white"
        );
        // The text line sits at y ~ (792 - 500) * 1.515 = 442, above the
        // baseline; any dark pixel there is a glyph the substitute font drew.
        let inked = (400..445)
            .flat_map(|y| (445..1154).map(move |x| (x, y)))
            .filter(|&(x, y)| pixels.get_pixel(x, y).0[0] < 0x40)
            .count();
        assert!(inked > 500, "the text is drawn, found {inked} dark pixels");
        let again = cover(FileKind::Pdf, &pdf).expect("a second page cover");
        assert_eq!(drawn, again, "the same PDF yields a byte-identical cover");
    }

    #[test]
    fn a_page_of_any_declared_size_renders_inside_the_frame() {
        let enormous = pdf_with("0 0 2000000 1500000", "0 0 1 rg 0 0 1000000 750000 re f");
        let drawn = cover(FileKind::Pdf, &enormous).expect("a page cover");
        assert_eq!(drawn.source, CoverSource::Page { index: 0 });
        assert_valid_cover(&drawn.image);

        // A sliver a hundred thousand times taller than it is wide still
        // lands in the frame, at least one pixel wide.
        let sliver = pdf_with("0 0 1 100000", "0 0 1 rg 0 0 1 100000 re f");
        let drawn = cover(FileKind::Pdf, &sliver).expect("a page cover");
        assert_eq!(drawn.source, CoverSource::Page { index: 0 });
        assert_valid_cover(&drawn.image);
    }

    #[test]
    fn a_bundles_pdf_is_drawn_when_the_bundle_carries_no_picture() {
        let bundle = zip_with(&[
            ("readme.txt", b"how to use this pack"),
            ("worksheet.pdf", &worksheet_pdf()),
        ]);
        let drawn = cover(FileKind::Zip, &bundle).expect("a bundle cover");
        assert_eq!(
            drawn.source,
            CoverSource::EmbeddedPage {
                path: "worksheet.pdf".to_owned(),
                index: 0
            },
            "the bundle's PDF is drawn and the cover names which entry it came from"
        );
        assert_valid_cover(&drawn.image);
    }

    #[test]
    fn a_small_preview_gives_way_to_a_bundled_page_and_a_large_one_does_not() {
        let small = zip_with(&[
            ("preview.png", &png_of(300, 200, MAGENTA)),
            ("worksheet.pdf", &worksheet_pdf()),
        ]);
        assert!(
            matches!(
                cover(FileKind::Zip, &small).expect("a cover").source,
                CoverSource::EmbeddedPage { .. }
            ),
            "a 300x200 preview is a thumbnail; the full-frame page is the better cover"
        );
        let large = zip_with(&[
            ("preview.png", &png_of(1200, 900, MAGENTA)),
            ("worksheet.pdf", &worksheet_pdf()),
        ]);
        assert_eq!(
            cover(FileKind::Zip, &large).expect("a cover").source,
            CoverSource::Embedded {
                path: "preview.png".to_owned()
            },
            "a preview sharp enough for the frame is the seller's chosen picture and wins"
        );
    }

    #[test]
    fn a_small_stored_thumbnail_is_shown_at_native_size_rather_than_stretched() {
        let document = docx_with(&[("docProps/thumbnail.jpeg", &jpeg_of(240, 180, MAGENTA))]);
        let drawn = cover(FileKind::Docx, &document).expect("a document cover");
        let pixels = decoded(&drawn.image);
        assert!(
            is_near(pixels.get_pixel(800 + 110, 600).0, MAGENTA),
            "inside the 240px-wide picture"
        );
        assert!(
            is_near(pixels.get_pixel(800 + 130, 600).0, GROUND),
            "just outside it is the light ground: the picture was not upscaled"
        );
    }

    #[test]
    fn the_card_is_a_neutral_grey_page_rather_than_a_red_block() {
        let card = cover(FileKind::Pdf, b"not a pdf").expect("the pdf card");
        let pixels = decoded(&card.image);
        assert!(
            pixels
                .pixels()
                .all(|pixel| pixel.0[0].abs_diff(pixel.0[2]) <= 16),
            "no pixel of the PDF card is red"
        );
        assert!(
            is_near(pixels.get_pixel(20, 20).0, [0xEC, 0xEC, 0xEE, 0xFF]),
            "the ground is light grey"
        );
        assert!(
            is_near(pixels.get_pixel(800, 900).0, [0xFF, 0xFF, 0xFF, 0xFF]),
            "a white page glyph sits on it"
        );
    }

    #[test]
    fn a_cover_whose_png_would_break_the_cap_is_stepped_down() {
        let mut state = 0x2545_F491_u32;
        let noise = RgbaImage::from_fn(COVER_WIDTH, COVER_HEIGHT, |_, _| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let [r, g, b, _] = state.to_le_bytes();
            Rgba([r, g, b, 0xFF])
        });
        let mut png = Vec::new();
        noise
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .expect("encode fixture");
        let drawn = cover(FileKind::Image, &png).expect("an image cover");
        assert!(
            drawn.image.png.len() <= COVER_BYTES_MAX,
            "the cover is under the cap, found {} bytes",
            drawn.image.png.len()
        );
        assert!(
            drawn.image.width >= 800 && drawn.image.width * 3 == drawn.image.height * 4,
            "and still 4:3 and no smaller than Tes's own cover, found {}x{}",
            drawn.image.width,
            drawn.image.height
        );
    }

    #[test]
    fn a_busy_page_renders_under_the_cover_cap() {
        // A worst case a PDF can realistically be: a landscape page tiled with
        // 4pt cells in unrelated colours, so almost no two neighbouring
        // pixels match and PNG's filters have nothing to work with.
        use std::fmt::Write as _;
        let mut state = 0x9E37_79B9_u32;
        let mut content = String::new();
        for row in 0..150u32 {
            for column in 0..200u32 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let [r, g, b, _] = state.to_le_bytes();
                writeln!(
                    content,
                    "{:.3} {:.3} {:.3} rg {} {} 4 4 re f",
                    f32::from(r) / 255.0,
                    f32::from(g) / 255.0,
                    f32::from(b) / 255.0,
                    column * 4,
                    row * 4,
                )
                .expect("writing to a String cannot fail");
            }
        }
        content.push_str("BT /F1 48 Tf 0 0 0 rg 40 280 Td (Busy page) Tj ET");
        let drawn = cover(FileKind::Pdf, &pdf_with("0 0 800 600", &content)).expect("a cover");
        assert_eq!(drawn.source, CoverSource::Page { index: 0 });
        assert!(
            drawn.image.png.len() <= COVER_BYTES_MAX,
            "a busy page is under the 2 MiB cap (and so under TPT's 4 MB), found {} bytes",
            drawn.image.png.len()
        );
        assert!(
            drawn.image.width >= 800 && drawn.image.width * 3 == drawn.image.height * 4,
            "downscaled to fit rather than cropped, found {}x{}",
            drawn.image.width,
            drawn.image.height
        );
    }

    #[test]
    fn the_tpt_square_pads_the_cover_rather_than_cropping_it() {
        let cover_png = png_of(COVER_WIDTH, COVER_HEIGHT, MAGENTA);
        let square = square_cover(&cover_png).expect("a square");
        assert_eq!(
            (square.width, square.height),
            (TPT_SQUARE_SIDE, TPT_SQUARE_SIDE)
        );
        let pixels = decoded(&square);
        assert_eq!(
            pixels.dimensions(),
            (TPT_SQUARE_SIDE, TPT_SQUARE_SIDE),
            "the PNG is the square it claims"
        );
        let white = [0xFF, 0xFF, 0xFF, 0xFF];
        for (x, y, wanted, why) in [
            (500, 500, MAGENTA, "the centre is the cover"),
            (
                0,
                500,
                MAGENTA,
                "the cover reaches the left edge: nothing was cropped",
            ),
            (999, 500, MAGENTA, "and the right edge"),
            (500, 125, MAGENTA, "a 1000x750 band starts at y 125"),
            (500, 874, MAGENTA, "and ends at y 874"),
            (500, 124, white, "white above the band"),
            (500, 875, white, "white below the band"),
        ] {
            assert!(
                is_near(pixels.get_pixel(x, y).0, wanted),
                "{why}: ({x}, {y}) is {:?}",
                pixels.get_pixel(x, y).0
            );
        }
    }
}
