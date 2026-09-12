//! The catalogue import's wire vocabulary: what one device saw of the
//! seller's own shop, in the shape it posts a page of it.
//!
//! Here rather than on the device for the reason the ledger vocabulary is
//! here: the device produces these and `tam-api` consumes them, and a
//! server-side struct that happens to match a client-side one today is a
//! coincidence rather than a contract. One definition means a field added to
//! either half fails to compile at the other rather than decoding to a
//! default nobody chose.
//!
//! # What the types refuse, stated exactly
//!
//! The threat model is a *modified* device, not merely a buggy one. The server
//! has never seen the machine that posts a page, so what a page is worth is
//! what its types refuse on the way in — which is the reasoning
//! [`Cover::try_from`] gives for re-checking a cover that is already
//! well-formed base64, and it applies to every field rather than to two of
//! them.
//!
//! No field of [`ObservedFile`] can carry a file: each is a closed enum, a
//! fixed-width digest, an integer, or a bounded string validated on decode.
//! That is a property of the types and holds against any producer.
//!
//! [`ObservedResource`] carries one thing that is deliberately not bounded
//! here: `listing`, whose title and body are the seller's own copy and must be
//! free text. Those are bounded by the catalogue's own limits where the route
//! writes them, and the honest statement of the page's guarantee is therefore
//! narrower than "there is nowhere to put a payload" — it is that every field
//! this vocabulary defines is bounded, and the one field it borrows is free
//! text by design.
//!
//! `ObservedFile::scan` is a closed enum whose `Infected` arm carries a
//! signature string owned by `tam_types`, shared with paths that predate this
//! one. It is bounded where the route validates a page rather than here,
//! because narrowing a type this crate does not own would reach further than
//! this vocabulary.
//!
//! # Compatibility
//!
//! Any field added to [`ImportPage`] or its parts after the first shipped
//! desktop takes `#[serde(default)]`, and a wire test asserts a page lacking
//! it still decodes. The direction that hurts is a newer server against an
//! older device: a required field added here makes an already-shipped device
//! fail to post after it has walked the seller's entire shop, fetched every
//! bundle and rendered every cover, with every retry failing identically. That
//! is the failure the 0.1.3 manifest shim exists to prevent, one layer up.
//!
//! There is deliberately no `deny_unknown_fields`. A newer device sending a
//! field this server does not know is a device that has been updated ahead of
//! us, and dropping the field is the outcome that lets the seller's migration
//! finish; `LedgerCall` chooses the opposite because a ledger call it cannot
//! fully understand is one it must not act on, and a page is not that.

use tam_marketplace::{ImportedListing, ListingState};
use tam_types::{ContentHash, FileKind, ScanOutcome, Uuid};

/// The sketch a device asserts, re-exported so a consumer of the page has one
/// path to it rather than a second dependency edge for one type.
pub use tam_fingerprint::{Fingerprint, TextSketch};

/// The longest name this vocabulary will carry for a file, in bytes.
///
/// Bytes rather than characters, because the storage column and every
/// downstream consumer measure bytes; a two-hundred-character bound over
/// astral-plane text is eight hundred bytes and the constant would be lying
/// about its own size.
pub const NAME_MAX: usize = 200;

/// The longest media type this vocabulary will carry. The longest real one is
/// the OpenXML presentation type at seventy-four bytes, so this is roughly
/// double the worst case rather than a guess.
pub const CONTENT_TYPE_MAX: usize = 127;

/// The longest marketplace resource address this vocabulary will carry. A
/// locator is an id or a URL the marketplace chose, so it is bounded rather
/// than shaped.
pub const LOCATOR_MAX: usize = 400;

/// The longest reason a device may give for skipping a resource. Long enough
/// for an adapter's own sentence, short enough that the field is a reason
/// rather than a channel.
pub const REASON_MAX: usize = 500;

/// The largest cover this vocabulary will carry.
///
/// The device's renderer produces a fixed 512 by 384 image, so anything
/// approaching this is not a cover.
pub const COVER_BYTES_MAX: usize = 512 * 1024;

/// A PNG's first eight bytes.
///
/// Public because the device's own content-type sniffing reads the same eight
/// bytes and should not keep a second copy. A third copy does exist, in
/// `tam-pipeline`'s probe; it stays there because this crate deliberately does
/// not depend on `tam-pipeline` — the crate documentation records that the
/// pipeline edge dragged an image decoder and a zip implementation into every
/// client binary — so consolidating onto it would cost more than the
/// duplication does.
pub const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Why a value a device was about to report is not one it may report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotReportable {
    NameTooLong(usize),
    NameNotAName,
    CoverNotPng,
    CoverTooLarge(usize),
    CoverNotBase64,
    ContentTypeTooLong(usize),
    ContentTypeNotAMediaType,
    LocatorTooLong(usize),
    LocatorNotALocator,
    ReasonTooLong(usize),
}

impl core::fmt::Display for NotReportable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NameTooLong(len) => {
                write!(
                    f,
                    "a file name is at most {NAME_MAX} bytes and this is {len}"
                )
            }
            Self::NameNotAName => f.write_str(
                "a file name is not empty, is not \".\" or \"..\", carries no path separator, no \
                 control character, no direction override and no byte-order mark, and does not \
                 begin or end in whitespace",
            ),
            Self::CoverNotPng => f.write_str("a cover is a PNG and this is not one"),
            Self::CoverTooLarge(len) => {
                write!(
                    f,
                    "a cover is at most {COVER_BYTES_MAX} bytes and this is {len}"
                )
            }
            Self::CoverNotBase64 => {
                f.write_str("a cover is canonical base64 and this is not well-formed")
            }
            Self::ContentTypeTooLong(len) => write!(
                f,
                "a media type is at most {CONTENT_TYPE_MAX} bytes and this is {len}"
            ),
            Self::ContentTypeNotAMediaType => {
                f.write_str("a media type is a type and a subtype separated by one solidus")
            }
            Self::LocatorTooLong(len) => write!(
                f,
                "a resource locator is at most {LOCATOR_MAX} bytes and this is {len}"
            ),
            Self::LocatorNotALocator => {
                f.write_str("a resource locator is not empty and carries no control character")
            }
            Self::ReasonTooLong(len) => {
                write!(
                    f,
                    "a reason is at most {REASON_MAX} bytes and this is {len}"
                )
            }
        }
    }
}

impl core::error::Error for NotReportable {}

/// A right-to-left or left-to-right override, and the byte-order mark.
///
/// Not control characters by Unicode's reckoning, and a spoofing device rather
/// than part of a name: an override in a displayed filename reverses what the
/// reader sees without changing what the bytes say.
fn is_spoofing(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// A file's own name, bounded and shaped.
///
/// A newtype rather than a `String` because of what the surrounding type has
/// to be able to promise. A page describes the seller's file and must not
/// contain it, and a bare string field is somewhere a payload can be encoded.
/// Validated on the way in and again on the way out of serde, so the promise
/// is the type's rather than the code's.
///
/// `.` and `..` are refused by name. Wherever a consumer joins an entry name
/// to a path or uses it as an object-store key, `..` is a traversal component,
/// and a type whose documentation says a name is not somewhere a file can be
/// hidden has to refuse it rather than rely on every consumer to.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FileName(String);

impl FileName {
    pub fn new(name: &str) -> Result<Self, NotReportable> {
        if name.len() > NAME_MAX {
            return Err(NotReportable::NameTooLong(name.len()));
        }
        let shaped = !name.is_empty()
            && name != "."
            && name != ".."
            && !name.contains(['/', '\\'])
            && !name.chars().any(char::is_control)
            && !name.chars().any(is_spoofing)
            && name.trim() == name;
        if !shaped {
            return Err(NotReportable::NameNotAName);
        }
        Ok(Self(name.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for FileName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for FileName {
    type Error = NotReportable;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        Self::new(&name)
    }
}

impl From<FileName> for String {
    fn from(name: FileName) -> Self {
        name.0
    }
}

/// The media type of the bytes handed onward, bounded and shaped.
///
/// A shape rather than the closed seven-value set the device's own sniffing
/// produces, deliberately: closing the set here would refuse a legitimate type
/// a later adapter learns to emit, and the property this field needs is that
/// it cannot carry a payload rather than that it is one of seven strings. The
/// storage column stays `text` and `FileBytes::Sourced` keeps its `String`;
/// the value is closed at the wire, where a modified device meets us, rather
/// than inside a frozen migration.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentType(String);

impl ContentType {
    pub fn new(value: &str) -> Result<Self, NotReportable> {
        if value.len() > CONTENT_TYPE_MAX {
            return Err(NotReportable::ContentTypeTooLong(value.len()));
        }
        let mut halves = value.split('/');
        let (Some(kind), Some(subtype), None) = (halves.next(), halves.next(), halves.next())
        else {
            return Err(NotReportable::ContentTypeNotAMediaType);
        };
        let token = |part: &str| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b))
        };
        if !token(kind) || !token(subtype) {
            return Err(NotReportable::ContentTypeNotAMediaType);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for ContentType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ContentType {
    type Error = NotReportable;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<ContentType> for String {
    fn from(value: ContentType) -> Self {
        value.0
    }
}

/// How a marketplace addresses one resource, bounded.
///
/// One type in both [`ObservedResource`] and [`SkippedResource`], because it is
/// one concept and the two travel in the same page. It was an `i64` on the skip
/// side, which could not express a skip for any resource whose address is not
/// numeric — and a Tes resource is addressed by a URL, so that was most of
/// them. A skip that cannot be represented defeats what the field is for: a
/// completing page that says nothing about what it failed on starts a publish
/// for a partial catalogue while reporting success.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Locator(String);

impl Locator {
    pub fn new(value: &str) -> Result<Self, NotReportable> {
        if value.len() > LOCATOR_MAX {
            return Err(NotReportable::LocatorTooLong(value.len()));
        }
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(NotReportable::LocatorNotALocator);
        }
        Ok(Self(value.to_owned()))
    }

    /// A locator from a marketplace's own numeric resource id.
    ///
    /// Infallible by construction rather than by a fallback nobody can reach:
    /// an `i64` renders as at most twenty ASCII digits and a sign, which is
    /// never empty, never a control character and never near the bound. A
    /// producer walking a numeric catalogue therefore has no error arm to
    /// invent a value for.
    #[must_use]
    pub fn from_resource_id(id: i64) -> Self {
        Self(id.to_string())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for Locator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Locator {
    type Error = NotReportable;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<Locator> for String {
    fn from(value: Locator) -> Self {
        value.0
    }
}

/// Why one resource could not be described, bounded.
///
/// Free-form prose from an adapter, so it is bounded rather than shaped:
/// control characters are allowed out of it because a marketplace's own error
/// text is not ours to reformat, and the length is what stops the field being
/// a channel.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Reason(String);

impl Reason {
    pub fn new(value: &str) -> Result<Self, NotReportable> {
        if value.len() > REASON_MAX {
            return Err(NotReportable::ReasonTooLong(value.len()));
        }
        Ok(Self(value.to_owned()))
    }

    /// The same, truncated on a character boundary rather than refused.
    ///
    /// For a producer turning an adapter's error into a reason: losing the tail
    /// of a message is better than losing the skip it explains, and the skip is
    /// what stops a partial catalogue publishing as a whole one.
    #[must_use]
    pub fn truncating(value: &str) -> Self {
        let mut used = 0usize;
        Self(
            value
                .chars()
                .take_while(|c| {
                    used += c.len_utf8();
                    used <= REASON_MAX
                })
                .collect(),
        )
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for Reason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Reason {
    type Error = NotReportable;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<Reason> for String {
    fn from(value: Reason) -> Self {
        value.0
    }
}

/// The derived cover, checked to be one.
///
/// Holds the decoded PNG rather than its encoding, which is what makes reading
/// the bytes back infallible by construction rather than by the good behaviour
/// of every present and future constructor. The wire form is base64, produced
/// on serialize and validated on deserialize, so a malformed encoding is
/// refused at the boundary and can never be stored or handed onward: an
/// encoding this codec reads happily and a browser's `atob` or Postgres's
/// `decode` rejects is a disagreement that would otherwise surface at whichever
/// consumer is furthest from here.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Cover(Vec<u8>);

impl Cover {
    /// Takes a rendered cover, refusing anything that is not one.
    pub fn encode(png: &[u8]) -> Result<Self, NotReportable> {
        if !png.starts_with(PNG_MAGIC) {
            return Err(NotReportable::CoverNotPng);
        }
        if png.len() > COVER_BYTES_MAX {
            return Err(NotReportable::CoverTooLarge(png.len()));
        }
        Ok(Self(png.to_vec()))
    }

    /// The PNG this cover carries. Infallible: the bytes are what the type
    /// holds, and both constructors checked them.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl TryFrom<String> for Cover {
    type Error = NotReportable;

    /// Re-checked on the way in, because a page arriving from anywhere but a
    /// device's own encoder has proved nothing by being well-formed base64.
    fn try_from(encoded: String) -> Result<Self, Self::Error> {
        let decoded = unbase64(&encoded).ok_or(NotReportable::CoverNotBase64)?;
        Self::encode(&decoded)
    }
}

impl From<Cover> for String {
    fn from(cover: Cover) -> Self {
        base64(&cover.0)
    }
}

/// What one device saw of one file, as the wire carries it.
///
/// Every field describes the bytes handed onward after the unwrap decision,
/// which is the same rule the storage columns and the payload manifest follow,
/// stated once here so the three cannot drift.
///
/// No field of this type can carry a file, and that is now a property of the
/// types rather than of the producer: the digest is a fixed-width hash, the
/// length is a number, the kind is a closed enum, the name and the media type
/// are bounded strings validated on decode, and the scan is a closed enum
/// whose one string arm the route bounds where it validates a page.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ObservedFile {
    pub payload_file_name: FileName,
    pub payload_content_type: ContentType,
    pub kind: FileKind,
    pub hash: ContentHash,
    pub byte_len: u64,
    pub scan: ScanOutcome,
    /// Which entry inside the bundle these bytes are, where the bundle reduced
    /// to one file. `None` is the bundle whole, and its absence is what says
    /// no unwrap happened.
    pub entry: Option<FileName>,
}

/// One resource, as one device read it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ObservedResource {
    /// How the seller's marketplace addresses it.
    pub locator: Locator,
    /// The listing verbatim. Its title and body are the seller's own copy and
    /// are free text by design; the route bounds them against the catalogue's
    /// own limits, which is where those limits are known.
    pub listing: ImportedListing,
    /// The file, where this device could fetch one.
    ///
    /// `None` is a source whose own-file download is uncaptured, which is
    /// TPT's measured state: the device read the listing and never held a
    /// byte of the resource. An absence rather than a zero-length file,
    /// because the two say different things to the duplicate matcher — no
    /// digest at all cannot fire L1, and a digest of nothing would fire it on
    /// every such resource at once.
    #[serde(default)]
    pub file: Option<ObservedFile>,
    /// The derived cover, checked to be one, riding in the page rather than
    /// going through the upload route.
    ///
    /// Absent exactly when `file` is: the cover is rendered from the payload,
    /// so a resource with no payload has no cover to render.
    #[serde(default)]
    pub cover_png: Option<Cover>,
    /// What this device measured of the file, for the duplicate matcher.
    ///
    /// Fixed-width and lossy by construction — see `tam-fingerprint` — so it
    /// describes the seller's document without carrying any of it, which is
    /// the same promise every other field of this type keeps. `None` is a
    /// device too old to measure one.
    #[serde(default)]
    pub fingerprint: Option<tam_fingerprint::Fingerprint>,
}

/// One resource as the enumeration saw it, before anything was read.
///
/// The selection step's whole content: the seller ticks from this list, and
/// only what they ticked is fetched. It is therefore deliberately cheap —
/// what a catalogue listing row already carries and nothing that needs a
/// second request per resource — because the alternative is walking the whole
/// shop twice to let the seller decline most of it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ListedResource {
    pub locator: Locator,
    /// The seller's own title, free text for the reason
    /// [`ObservedResource::listing`]'s is.
    pub title: String,
    /// The price in the smallest unit of `currency`, where the row carried
    /// one. Absent is "the enumeration did not say", never "free".
    pub price_minor: Option<i64>,
    pub currency: Option<String>,
    /// Whether the source calls it a draft, where the row said. A draft has
    /// no published file, so the selection step can grey it out rather than
    /// letting the seller pick something that will skip.
    pub state: Option<ListingState>,
}

/// One page of the catalogue, as a device posts it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImportPage {
    /// Which import run this belongs to, so a resumed pass is the same run
    /// rather than a second one.
    ///
    /// Every import page names one. The `request` beside it is the older
    /// anchor and is the migration path's alone.
    pub run: Uuid,
    /// The sync request this page belongs to, for the device-enumerated
    /// migrate branch of `POST /v1/sync`.
    ///
    /// `serde(default)` and absent on every import page: an import is a run,
    /// and a page names one or the other.
    #[serde(default)]
    pub request: Option<Uuid>,
    /// The shop as the enumeration saw it, on the one page that carries it.
    ///
    /// The first page of a run and no other. It is what the selection step
    /// renders, and its length is the run's `read_total` — which is why it is
    /// `Option` rather than an empty vector: a shop that holds nothing and an
    /// ordinary resources page must not read alike, or a run would report a
    /// total of nothing and settle before the seller saw it.
    #[serde(default)]
    pub listed: Option<Vec<ListedResource>>,
    pub resources: Vec<ObservedResource>,
    /// What this page could not describe, and why.
    ///
    /// Travels with the page rather than staying on the device, because
    /// completion is what mints the write jobs: a completing page that said
    /// nothing about the resources it failed on would start a publish for a
    /// partial catalogue while reporting success, and the seller would find
    /// out by noticing something missing from their own shop. The server
    /// records these against the request so the console can show what did not
    /// cross and the seller can decide whether to proceed.
    pub skipped: Vec<SkippedResource>,
    /// Whether this is the last page. The server mints the write jobs when it
    /// is, which is what makes the device saying "complete" the thing that
    /// starts the publish rather than a separate action nobody took.
    pub complete: bool,
    /// The import stopped, and this is why.
    ///
    /// The seller's own run page is the record they read, so a failure that
    /// posted nothing has to reach it or it reaches nobody: a first page that
    /// could not be sent, or a sign-out mid-pass, leaves the run holding
    /// exactly nothing and the console watching a state that never changes. A
    /// page carrying this settles the run failed with this sentence in its
    /// `failure_detail`, which the run page already renders.
    ///
    /// Completion is implied rather than stated. A stopped import is over,
    /// and a page that said `failed` and `complete: false` would be asking the
    /// server to hold a run open for work that has ended.
    ///
    /// `serde(default)` because it was added after the first shipped desktop,
    /// which is the convention this module's documentation states: a device
    /// that does not know the field posts pages without it and the server
    /// reads them unchanged.
    #[serde(default)]
    pub failed: Option<Reason>,
}

/// Why one resource could not be described.
///
/// Carried per resource rather than failing the pass, because one unreadable
/// resource in a shop of hundreds should cost that resource rather than the
/// migration. The pass reports them and continues.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkippedResource {
    pub locator: Locator,
    pub why: Reason,
}

/// Base64, standard alphabet with padding, written here rather than taken as a
/// dependency: one cover per resource is the only thing this vocabulary
/// encodes, and an edge for twenty lines is a poor trade.
#[must_use]
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let bits = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for slot in 0..4 {
            if slot <= chunk.len() {
                let index = (bits >> (18 - 6 * slot)) & 0x3F;
                out.push(char::from(ALPHABET[index as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Decodes canonical standard base64 with padding. `None` for anything else.
///
/// Strict on three counts a permissive decoder lets through, each of which was
/// observed accepted by the version this replaces. Padding appears only in the
/// final chunk, so `"Zg==Zm9v"` is refused rather than decoding to `"ffoo"`.
/// A value character after a pad within a chunk is refused, so `"AB=C"` no
/// longer yields the same bytes as `"AB=="` plus a silently discarded
/// character. And the bits a pad makes unused must be zero, so `"Zh=="` is
/// refused where `"Zg=="` is accepted, which is what makes an encoding round
/// trip to itself.
///
/// Strictness is the point rather than pedantry: [`Cover`] holds decoded bytes
/// and re-encodes them, so anything this accepts is something we will hand
/// onward as canonical, and a decoder that is more permissive than the
/// consumers downstream moves the disagreement to whichever of them is
/// furthest away.
pub(crate) fn unbase64(text: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let raw = text.as_bytes();
    if raw.is_empty() {
        return Some(Vec::new());
    }
    if !raw.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(raw.len());
    for (index, chunk) in raw.chunks(4).enumerate() {
        let last = (index + 1) * 4 == raw.len();
        // Padding is trailing by construction: wherever the first pad is, every
        // byte from there on must be one, which is what refuses "AB=C" — a
        // value character after a pad, whose bits were previously read and then
        // silently discarded.
        let first_pad = chunk.iter().position(|byte| *byte == b'=').unwrap_or(4);
        let pads = 4 - first_pad;
        if pads > 0 && (!last || pads > 2 || !chunk[first_pad..].iter().all(|byte| *byte == b'=')) {
            return None;
        }
        let kept = 3 - pads;
        let mut bits: u32 = 0;
        for (slot, byte) in chunk.iter().take(first_pad).enumerate() {
            let value = ALPHABET.iter().position(|candidate| candidate == byte)?;
            bits |= u32::try_from(value).ok()? << (18 - 6 * slot);
        }
        // The bits below the bytes we keep are carried by no output byte, so a
        // canonical encoding leaves them zero. "Zh==" and "Zg==" would
        // otherwise decode alike while only one re-encodes to itself.
        let unused = (1u32 << (8 * (3 - kept))) - 1;
        if bits & unused != 0 {
            return None;
        }
        out.extend_from_slice(&bits.to_be_bytes()[1..=kept]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{base64, unbase64, Cover, FileName, NAME_MAX};

    #[test]
    fn base64_matches_the_standard_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn decoding_inverts_encoding_over_every_padding_case() {
        for original in [
            b"".as_slice(),
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
        ] {
            assert_eq!(unbase64(&base64(original)).as_deref(), Some(original));
        }
    }

    /// The cases a permissive decoder accepts, every one of them an input the
    /// review observed the previous version decoding rather than refusing.
    #[test]
    fn a_malformed_encoding_is_none_rather_than_a_guess() {
        assert_eq!(
            unbase64("Zg="),
            None,
            "a length that is not a multiple of 4"
        );
        assert_eq!(unbase64("Zm9v!!!!"), None, "a byte outside the alphabet");
        assert_eq!(unbase64("=Zm8"), None, "padding where a value belongs");

        assert_eq!(
            unbase64("Zg==Zm9v"),
            None,
            "padding in a non-final chunk decoded to \"ffoo\" before this"
        );
        assert_eq!(unbase64("Zg==Zg=="), None, "and so did two padded chunks");
        assert_eq!(
            unbase64("AB=C"),
            None,
            "a value after a pad decoded to the same bytes as AB== plus a discarded character"
        );
        assert_eq!(unbase64("AB=A"), None, "the same shape, a different tail");

        assert_eq!(unbase64("Zg=="), Some(b"f".to_vec()), "the canonical one");
        assert_eq!(
            unbase64("Zh=="),
            None,
            "and its non-canonical twins, which decoded alike and re-encoded to something else"
        );
        assert_eq!(unbase64("Zi=="), None);
    }

    #[test]
    fn a_cover_holds_its_bytes_so_reading_them_back_cannot_fail() {
        let mut png = super::PNG_MAGIC.to_vec();
        png.extend_from_slice(b"IHDR");
        let cover = Cover::encode(&png).expect("the fixture is a PNG");
        assert_eq!(cover.bytes(), png.as_slice());

        let wire = String::from(cover.clone());
        let back: Cover = wire.try_into().expect("the encoding round trips");
        assert_eq!(back, cover, "and the wire form is the bytes, not a copy");
    }

    #[test]
    fn a_file_name_refuses_what_is_not_a_name() {
        for refused in [
            "",
            ".",
            "..",
            "  ",
            " leading.pdf",
            "trailing.pdf ",
            "a\u{202E}b.pdf",
            "\u{FEFF}x.pdf",
            "dir/file.pdf",
            "dir\\file.pdf",
            "line\nbreak.pdf",
        ] {
            assert!(
                FileName::new(refused).is_err(),
                "{refused:?} is not a name and the type has to say so"
            );
        }
        assert!(FileName::new("worksheet.pdf").is_ok());
        assert!(
            FileName::new("C:file.pdf").is_ok(),
            "a colon is not a separator on the platforms this runs on, and refusing it would \
             refuse a legitimate marketplace name"
        );
    }

    #[test]
    fn a_file_name_is_bounded_in_bytes_rather_than_characters() {
        let astral = "\u{1F600}".repeat(NAME_MAX);
        assert!(
            FileName::new(&astral).is_err(),
            "{NAME_MAX} astral characters is four times {NAME_MAX} bytes, and the column that \
             stores this counts bytes"
        );
        assert!(FileName::new(&"n".repeat(NAME_MAX)).is_ok());
        assert!(FileName::new(&"n".repeat(NAME_MAX + 1)).is_err());
    }

    #[test]
    fn a_media_type_is_a_type_and_a_subtype() {
        use super::{ContentType, CONTENT_TYPE_MAX};
        assert!(ContentType::new("application/pdf").is_ok());
        assert!(ContentType::new(
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        )
        .is_ok());
        for refused in [
            "",
            "application",
            "application/",
            "/pdf",
            "a/b/c",
            "app lication/pdf",
        ] {
            assert!(
                ContentType::new(refused).is_err(),
                "{refused:?} is not a media type"
            );
        }
        assert!(
            ContentType::new(&"a".repeat(CONTENT_TYPE_MAX + 1)).is_err(),
            "and the bound holds before the shape is even considered"
        );
    }

    /// The compatibility rule this module's documentation states, exercised
    /// on every field added since the first shipped desktop. A page that
    /// names a run, its resources and nothing else must decode.
    #[test]
    fn a_page_without_any_of_the_added_fields_still_decodes() {
        let minimal = r#"{
            "run": "00000000-0000-0000-0000-000000000001",
            "resources": [],
            "skipped": [],
            "complete": false
        }"#;
        let page: super::ImportPage =
            serde_json::from_str(minimal).expect("a minimal page decodes");
        assert_eq!(page.request, None);
        assert_eq!(page.listed, None);
        assert_eq!(page.failed, None);
        assert!(page.resources.is_empty());
    }

    /// And the same for the resource: a source that could fetch no file
    /// names none, rather than being unrepresentable.
    #[test]
    fn a_resource_with_no_file_decodes_as_one() {
        let fileless = r#"{
            "locator": "12345",
            "listing": {
                "remote": {"tpt": {"product_id": 12345}},
                "title": "Fractions on a number line",
                "body": "",
                "body_format": "Markdown",
                "native": [],
                "rights": null,
                "price": {"paid": {"minor_units": 450, "denomination": "USD"}},
                "state": "live"
            }
        }"#;
        let observed: super::ObservedResource =
            serde_json::from_str(fileless).expect("a fileless resource decodes");
        assert_eq!(observed.file, None);
        assert_eq!(observed.cover_png, None);
        assert_eq!(observed.fingerprint, None);
    }

    /// An empty shop and a page that simply carries no listing are different
    /// facts, and the wire keeps them apart.
    #[test]
    fn an_empty_enumeration_is_not_an_absent_one() {
        let empty = r#"{
            "run": "00000000-0000-0000-0000-000000000001",
            "listed": [],
            "resources": [],
            "skipped": [],
            "complete": true
        }"#;
        let page: super::ImportPage = serde_json::from_str(empty).expect("an empty shop decodes");
        assert_eq!(
            page.listed,
            Some(Vec::new()),
            "a shop that holds nothing said so"
        );
    }
}
