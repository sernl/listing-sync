//! What one device measured of one resource's file, in the shape the
//! duplicate matcher compares.
//!
//! Its own crate rather than a module of `tam-pipeline`, and the reason is a
//! dependency edge rather than tidiness: the pipeline is on the portable leg
//! of `just check-portable`, and a PDF text extractor, a PDF object parser and
//! a perceptual hasher would then have to compile for five client targets that
//! never read a file. Here they sit behind the `compute` feature, and the half
//! that crosses the wire — the sketch types and the comparisons over them —
//! carries no reader at all.
//!
//! # What a fingerprint is, and is not
//!
//! It is an assertion by a device about bytes the server never held, in the
//! voice `0052_product_file_source.sql` uses for exactly that. Nothing here is
//! a verification; the server stores what a device claimed and compares claims
//! with claims.
//!
//! It is also fixed-width and lossy by construction. The text layer is a
//! 64-bit SimHash and a 128-hash MinHash over 5-word shingles — 520 bytes for
//! a document of any length — and neither can be read back into text. That is
//! the property that lets a device send it: the seller's words stay on the
//! seller's machine, and what crosses is a number that answers "is this the
//! same document" and no other question.
//!
//! # Which layer each field feeds
//!
//! - `text` → L2, the layer that survives a re-export. `simhash` is the
//!   blocker (Hamming at most 3 over the banded index); `minhash` is the
//!   score (Jaccard).
//! - `page_count` → L5's equal-page-count conjunct, and the strong negative
//!   at more than 20% apart. Read from the PDF page tree, because no
//!   marketplace read carries it.
//! - `cover_phash` → L3, and only for a payload that is itself an image.
//!   `tam_pipeline::render::cover` draws a flat kind-coloured card for a PDF,
//!   a PPTX, a DOCX and a ZIP, so a pHash over one of those compares
//!   constants: every PDF in the org would hash identically and L3 would fire
//!   on every pair. Computing it only for `FileKind::Image` is what stops a
//!   layer that measures nothing from voting.
//! - `title_norm` → L4, and the trigram blocking index.
//!
//! # Versioning
//!
//! [`FINGERPRINT_VERSION`] freezes the whole shape: the shingle width, the
//! token rule, the permutation keys, the bit order, and which kinds are read.
//! Sketches are never compared across versions — a bump is a recompute on the
//! device, not a migration of stored numbers — so any change to the functions
//! below that could move one bit is a bump.

use serde::{Deserialize, Serialize};
#[cfg(feature = "compute")]
use tam_types::FileKind;

/// The format this crate produces and compares.
///
/// Bumped whenever a byte of any sketch could move: the shingle width, the
/// tokenisation, the permutation keys, the bit order, or which file kinds are
/// read at all.
pub const FINGERPRINT_VERSION: u16 = 1;

/// How many hashes a MinHash sketch carries.
pub const MINHASH_K: usize = 128;

/// The MinHash sketch's width in bytes, which is what the storage column and
/// the wire both measure.
pub const MINHASH_BYTES: usize = MINHASH_K * 4;

/// How many words one shingle spans.
///
/// Five, from the research: short enough that a re-export which reflows a
/// paragraph keeps most shingles, long enough that two unrelated worksheets
/// on the same topic share almost none.
pub const SHINGLE_WORDS: usize = 5;

/// The largest file this crate will read for text.
///
/// A cap rather than a budget extension: a PDF past this is a scanned book or
/// a video, and either way the extraction is minutes of work for a layer that
/// a scanned document cannot feed anyway. Past it the text layer is absent,
/// which is a measured absence and not a zero.
pub const TEXT_BYTES_MAX: usize = 32 * 1024 * 1024;

/// How long the PDF read may take before the text layer is given up on.
///
/// The device holds the seller's whole import open behind this, one resource
/// at a time, so an extractor that wanders into a pathological object graph
/// must cost this resource's text rather than the import.
pub const TEXT_BUDGET: core::time::Duration = core::time::Duration::from_mins(1);

/// The longest normalised title this crate will report.
///
/// `product_fingerprint.title_norm` is bounded at four hundred characters, so
/// a longer one is a value that cannot be stored; truncating here means the
/// device reports something the server can write rather than a row that fails
/// its own CHECK at the end of an import.
pub const TITLE_NORM_MAX: usize = 400;

/// The text layer, as a device measured it.
///
/// Present or absent as one measurement: there is no partial text sketch, and
/// `extracted_chars == 0` is a PDF that is pages of scanned images rather
/// than a PDF that could not be read. The two are different facts and the
/// matcher treats them differently, which is why the zero is reported rather
/// than collapsed into absence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSketch {
    /// The 64-bit SimHash over the same shingles, which is the blocking key.
    pub simhash: u64,
    /// 128 minima, one per permutation. Base64 of 512 little-endian bytes on
    /// the wire and in the `bytea` column, so the two spellings are one.
    #[serde(with = "minhash_wire")]
    pub minhash: [u32; MINHASH_K],
    /// How many distinct shingles the text yielded. Zero for text shorter
    /// than one shingle, which is a document too short for L2 to mean
    /// anything and is reported rather than hidden.
    pub shingle_count: u32,
    /// How many characters the extractor returned, before shingling.
    pub extracted_chars: u32,
}

/// Everything one device measured of one resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    /// Which format produced it. Carried rather than assumed, because a
    /// server holding sketches of two versions must refuse to compare them
    /// rather than compare them badly.
    pub version: u16,
    /// The text layer, for a PDF that could be read.
    pub text: Option<TextSketch>,
    /// Pages, from the PDF page tree.
    pub page_count: Option<u32>,
    /// The perceptual hash of the cover, for an image payload only.
    pub cover_phash: Option<u64>,
    /// The listing's own title, normalised. Always present: every listing has
    /// a title, though a title of nothing but punctuation normalises to the
    /// empty string, which is reported as it is rather than invented into
    /// something.
    pub title_norm: String,
}

impl Fingerprint {
    /// What a source with no file can assert.
    ///
    /// TPT's own-file download is uncaptured, so a TPT-sourced resource has no
    /// bytes on this device at all: no digest, no text, no page count, and no
    /// cover to hash. The honest fingerprint is the title and four measured
    /// absences, which is what confines a TPT pair to L4 and L5 and keeps it
    /// out of every auto-merge.
    #[must_use]
    pub fn of_title(title: &str) -> Self {
        Self {
            version: FINGERPRINT_VERSION,
            text: None,
            page_count: None,
            cover_phash: None,
            title_norm: normalise_title(title),
        }
    }
}

/// Everything this device can measure of one resource's payload.
///
/// `cover_png` is the rendered cover the import pass already holds, passed in
/// rather than rendered here: this crate does not depend on the renderer, and
/// the pass has the image in hand.
#[cfg(feature = "compute")]
#[must_use]
pub fn fingerprint(
    kind: FileKind,
    bytes: &[u8],
    title: &str,
    cover_png: Option<&[u8]>,
) -> Fingerprint {
    let (text, page_count) = match kind {
        FileKind::Pdf => read_pdf(bytes),
        FileKind::Pptx | FileKind::Docx | FileKind::Zip | FileKind::Image => (None, None),
    };
    // Only an image payload: see the crate documentation on why a pHash over
    // a placeholder card is a layer that fires on everything.
    let cover_phash = match kind {
        FileKind::Image => cover_png.and_then(phash),
        FileKind::Pdf | FileKind::Pptx | FileKind::Docx | FileKind::Zip => None,
    };
    Fingerprint {
        version: FINGERPRINT_VERSION,
        text,
        page_count,
        cover_phash,
        title_norm: normalise_title(title),
    }
}

/// The text sketch and the page count, both under one budget on one thread.
///
/// A thread rather than an in-line call, because neither reader can be
/// interrupted and both can run away on a malformed file: the budget is only
/// enforceable by abandoning the worker. It is also where the unwind guard
/// sits — a PDF parser reached by an untrusted file is exactly the place a
/// panic arrives, and a panicking import would cost the seller the whole run
/// rather than one resource's text layer.
#[cfg(feature = "compute")]
// The one place in this crate that may contain a panic, and it is the worker
// boundary the lint's reason names: a dedicated thread whose whole job is
// reading one untrusted PDF, whose panic must cost that file's text layer and
// nothing else. `pdf-extract` indexes and unwraps its way through a document
// an adversary supplied; without this, one malformed file ends the seller's
// whole import.
#[expect(
    clippy::disallowed_methods,
    reason = "this thread is the worker boundary the ban reserves it for"
)]
fn read_pdf(bytes: &[u8]) -> (Option<TextSketch>, Option<u32>) {
    if bytes.len() > TEXT_BYTES_MAX {
        return (None, None);
    }
    // Owned because the worker outlives this call when the budget runs out.
    let owned = bytes.to_vec();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("tam-fingerprint-pdf".to_owned())
        .spawn(move || {
            let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let text = pdf_extract::extract_text_from_mem(&owned)
                    .ok()
                    .map(|text| sketch_of(&text));
                let pages = lopdf::Document::load_mem(&owned)
                    .ok()
                    .map(|doc| u32::try_from(doc.get_pages().len()).unwrap_or(u32::MAX));
                (text, pages)
            }))
            .unwrap_or((None, None));
            drop(tx.send(read));
        });
    // A thread that could not be spawned is the same outcome as one that ran
    // out of budget: no text layer, reported as absent.
    if spawned.is_err() {
        return (None, None);
    }
    rx.recv_timeout(TEXT_BUDGET).unwrap_or((None, None))
}

/// The perceptual hash of a cover, as a `u64`.
///
/// `HashAlg::Mean` rather than a DCT-based pHash, deliberately. `image_hasher`
/// calls it the mean hash and it is blockhash-adjacent: it downsamples to the
/// hash grid and compares each cell to the image's mean, so it is stable under
/// recompression, rescaling and the mild colour shifts a marketplace's own
/// thumbnailer applies — which is every difference a cross-marketplace pair
/// actually exhibits. The DCT variants buy robustness to rotation and gamma
/// that no marketplace introduces, at a cost this device pays per resource.
/// The grid is the default 8 by 8, which is the sixty-four bits `cover_phash`
/// carries.
#[cfg(feature = "compute")]
fn phash(png: &[u8]) -> Option<u64> {
    let decoded = image::load_from_memory(png).ok()?;
    let hasher = image_hasher::HasherConfig::new()
        .hash_alg(image_hasher::HashAlg::Mean)
        .to_hasher();
    let hash = hasher.hash_image(&decoded);
    let bytes: [u8; 8] = hash.as_bytes().try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

/// The sketch of one extracted text.
///
/// Public because the server-side spreadsheet path holds the bytes itself and
/// needs the same numbers from the same code; a second implementation there
/// would be a second format wearing one version number.
#[must_use]
pub fn sketch_of(text: &str) -> TextSketch {
    let shingles = shingles(text);
    TextSketch {
        simhash: simhash(&shingles),
        minhash: minhash(&shingles),
        shingle_count: u32::try_from(shingles.len()).unwrap_or(u32::MAX),
        extracted_chars: u32::try_from(text.chars().count()).unwrap_or(u32::MAX),
    }
}

/// The 5-word shingles of one text, deduplicated.
///
/// Lowercased with punctuation stripped and whitespace collapsed, so a
/// re-export that changed the line breaks or the quote characters produces the
/// same shingles. Deduplicated because both sketches are over a *set*: a
/// document repeating one phrase thirty times must not weight it thirty times
/// in the SimHash, and MinHash is defined over sets in any case.
fn shingles(text: &str) -> Vec<String> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect();
    let mut seen: Vec<String> = words
        .windows(SHINGLE_WORDS)
        .map(|window| window.join(" "))
        .collect();
    seen.sort_unstable();
    seen.dedup();
    seen
}

/// The 32-byte blake3 key for one permutation.
///
/// The permutation index in little-endian bytes and zeroes after it, which is
/// the whole derivation: 128 keyed hashes of one shingle are 128 independent
/// hash functions of it, which is what MinHash needs, and deriving the keys
/// from the index alone means the sketch depends on nothing but the text and
/// this constant.
fn permutation_key(index: usize) -> [u8; 32] {
    let mut key = [0u8; 32];
    let index = u16::try_from(index).unwrap_or(u16::MAX);
    key[..2].copy_from_slice(&index.to_le_bytes());
    key
}

/// The MinHash sketch: the least hash of any shingle, per permutation.
///
/// An empty shingle set yields `u32::MAX` in every slot, which compares equal
/// to another empty set and near-zero to anything else. That is the right
/// answer rather than a degenerate one: two documents that yielded no text
/// are not thereby the same document, and `shingle_count == 0` is what the
/// matcher reads to refuse the layer.
fn minhash(shingles: &[String]) -> [u32; MINHASH_K] {
    let mut sketch = [u32::MAX; MINHASH_K];
    for shingle in shingles {
        for (slot, index) in sketch.iter_mut().zip(0..MINHASH_K) {
            let hashed = blake3::keyed_hash(&permutation_key(index), shingle.as_bytes());
            let head: [u8; 4] = [
                hashed.as_bytes()[0],
                hashed.as_bytes()[1],
                hashed.as_bytes()[2],
                hashed.as_bytes()[3],
            ];
            *slot = (*slot).min(u32::from_le_bytes(head));
        }
    }
    sketch
}

/// The 64-bit SimHash over the same shingles.
///
/// Each shingle votes on each of sixty-four bits by its own blake3, the votes
/// are summed, and the sign of each sum is the bit. Unweighted, because every
/// shingle of a worksheet is as much evidence as every other and an IDF
/// weighting would need a corpus this device does not have.
fn simhash(shingles: &[String]) -> u64 {
    let mut votes = [0i32; 64];
    for shingle in shingles {
        let hashed = blake3::hash(shingle.as_bytes());
        let head: [u8; 8] = [
            hashed.as_bytes()[0],
            hashed.as_bytes()[1],
            hashed.as_bytes()[2],
            hashed.as_bytes()[3],
            hashed.as_bytes()[4],
            hashed.as_bytes()[5],
            hashed.as_bytes()[6],
            hashed.as_bytes()[7],
        ];
        let bits = u64::from_le_bytes(head);
        for (bit, vote) in votes.iter_mut().enumerate() {
            *vote += if bits >> bit & 1 == 1 { 1 } else { -1 };
        }
    }
    let mut hash = 0u64;
    for (bit, vote) in votes.iter().enumerate() {
        if *vote > 0 {
            hash |= 1 << bit;
        }
    }
    hash
}

/// A title with everything a marketplace added to it taken off.
///
/// Lowercased, marketplace boilerplate removed, punctuation dropped, spaces
/// collapsed. What is *not* removed is the part that matters: "distance
/// learning", "printable", "digital", "no prep" and "bundle" stay, because
/// they are the seller's own words and two listings differing only by
/// "bundle" are very plausibly different products. Only the suffixes a
/// marketplace appends to its own listing titles go — `| TPT`, `- TPT`,
/// `| Tes`, `(Tes)`, `(TPT)` — and they go before the punctuation strip,
/// because the punctuation is what identifies them.
#[must_use]
pub fn normalise_title(title: &str) -> String {
    let mut text = title.to_lowercase();
    for marker in ["| tpt", "- tpt", "| tes", "(tes)", "(tpt)"] {
        text = text.replace(marker, " ");
    }
    let stripped: String = text
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let collapsed = stripped.split_whitespace().collect::<Vec<&str>>().join(" ");
    collapsed.chars().take(TITLE_NORM_MAX).collect()
}

/// The four 16-bit bands of a SimHash, which are the blocking keys.
///
/// Manku's rule: two hashes within Hamming distance three must agree on at
/// least one of four disjoint bands, so four equality indexes find every
/// candidate a linear scan would and the server never scans. Least-significant
/// band first, which is the order the four stored columns are numbered in.
#[must_use]
// The truncation is the operation: a band IS the low sixteen bits of its
// shifted hash, so `try_from` here would be asking whether a value that
// cannot be out of range is out of range.
#[expect(
    clippy::cast_possible_truncation,
    reason = "each band is exactly the low sixteen bits of its shift"
)]
pub const fn simhash_bands(hash: u64) -> [u16; 4] {
    [
        hash as u16,
        (hash >> 16) as u16,
        (hash >> 32) as u16,
        (hash >> 48) as u16,
    ]
}

/// The estimated Jaccard similarity of two shingle sets, from their sketches.
///
/// The fraction of permutations on which the two minima agree, which is an
/// unbiased estimator of the Jaccard index with a standard error of about
/// 1/sqrt(128), near nine percent. That is why the L2 thresholds are 0.9 and
/// 0.6 rather than anything finer: the estimator cannot tell 0.86 from 0.9.
#[must_use]
pub fn minhash_jaccard(left: &[u32; MINHASH_K], right: &[u32; MINHASH_K]) -> f32 {
    let agreed = left
        .iter()
        .zip(right.iter())
        .filter(|(one, other)| one == other)
        .count();
    ratio(agreed, MINHASH_K)
}

/// How many bits two hashes differ in.
#[must_use]
pub const fn hamming(left: u64, right: u64) -> u32 {
    (left ^ right).count_ones()
}

/// The Jaccard similarity of two normalised titles, over their word sets.
///
/// Both sides are normalised again here rather than assumed normalised: the
/// caller compares a stored `title_norm` against one it just computed, and a
/// function whose answer depends on whether its caller remembered is a
/// function that will eventually be called wrong.
#[must_use]
pub fn token_jaccard(left: &str, right: &str) -> f32 {
    let one = token_set(left);
    let other = token_set(right);
    let shared = one.iter().filter(|token| other.contains(token)).count();
    ratio(shared, one.len() + other.len() - shared)
}

/// A count over a count, as an `f32`, with no cast anywhere near it.
///
/// Both arguments are bounded small — 128 permutations, a few hundred title
/// tokens — so the widening is exact; `u16` is where that is stated rather
/// than assumed, and a denominator of nothing scores nothing rather than
/// dividing by zero.
fn ratio(part: usize, whole: usize) -> f32 {
    if whole == 0 {
        return 0.0;
    }
    let part = u16::try_from(part).unwrap_or(u16::MAX);
    let whole = u16::try_from(whole).unwrap_or(u16::MAX);
    f32::from(part) / f32::from(whole)
}

fn token_set(title: &str) -> Vec<String> {
    let mut tokens: Vec<String> = normalise_title(title)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    tokens.sort_unstable();
    tokens.dedup();
    tokens
}

/// The 512 bytes a MinHash sketch is, in the one order every consumer reads.
///
/// Little-endian per hash, in permutation order. The `bytea` column, the wire
/// base64 and this function are the same bytes; there is deliberately no
/// second spelling anywhere.
#[must_use]
pub fn minhash_bytes(sketch: &[u32; MINHASH_K]) -> [u8; MINHASH_BYTES] {
    let mut bytes = [0u8; MINHASH_BYTES];
    for (slot, hash) in bytes.chunks_exact_mut(4).zip(sketch.iter()) {
        slot.copy_from_slice(&hash.to_le_bytes());
    }
    bytes
}

/// The inverse, refusing anything that is not exactly 512 bytes.
#[must_use]
pub fn minhash_from_bytes(bytes: &[u8]) -> Option<[u32; MINHASH_K]> {
    if bytes.len() != MINHASH_BYTES {
        return None;
    }
    let mut sketch = [0u32; MINHASH_K];
    for (slot, chunk) in sketch.iter_mut().zip(bytes.chunks_exact(4)) {
        let head: [u8; 4] = chunk.try_into().ok()?;
        *slot = u32::from_le_bytes(head);
    }
    Some(sketch)
}

/// Base64 of the 512 bytes, both ways, so JSON and Postgres carry one value.
mod minhash_wire {
    use super::{minhash_bytes, minhash_from_bytes, MINHASH_K};
    use base64::Engine as _;
    use serde::{Deserialize as _, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        sketch: &[u32; MINHASH_K],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(minhash_bytes(sketch));
        serializer.serialize_str(&encoded)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[u32; MINHASH_K], D::Error> {
        let encoded = String::deserialize(deserializer)?;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .map_err(serde::de::Error::custom)?;
        minhash_from_bytes(&decoded).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "a minhash sketch is {} bytes and this one is {}",
                super::MINHASH_BYTES,
                decoded.len()
            ))
        })
    }
}

#[cfg(test)]
mod tests;
