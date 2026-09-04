//! The pure vocabulary and wire ADTs shared by every layer of the workspace.
//!
//! Every definition is promoted verbatim from `docs/design/sketches/domain.rs`,
//! the artefact of record for the domain types. The crate carries no I/O
//! dependency: `serde` only, and deserialisation routes through the smart
//! constructor wherever a type carries an invariant, so a wire value cannot
//! reach a state the constructor would refuse.

#![forbid(unsafe_code)]

pub mod actor;
pub mod connection;
pub mod natives;

pub use actor::{Actor, Stamp, SystemComponent};
pub use connection::{
    connection_status, ConnectionEvent, ConnectionHealth, ConnectionState, ConnectionStatus,
    VERIFICATION_FRESHNESS_MS,
};
use serde::{Deserialize, Serialize};

/// Stands in for the `uuid` crate's type. On the wire it is the canonical
/// lowercase hyphenated string, not a byte array: identifiers cross the API
/// to a web client, and the client's form is the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uuid(pub [u8; 16]);

impl Uuid {
    /// The canonical 8-4-4-4-12 lowercase form.
    #[must_use]
    pub fn to_hyphenated(&self) -> String {
        use core::fmt::Write;
        let mut out = String::with_capacity(36);
        for (index, byte) in self.0.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                out.push('-');
            }
            // infallible on String; the Result is the trait's, not the writer's
            let _unused: core::fmt::Result = write!(out, "{byte:02x}");
        }
        out
    }

    /// Parses the hyphenated form (either case); anything else is `None`.
    #[must_use]
    pub fn parse_hyphenated(raw: &str) -> Option<Self> {
        let bytes_hex: Vec<u8> = raw.bytes().filter(|byte| *byte != b'-').collect();
        if raw.len() != 36 || bytes_hex.len() != 32 {
            return None;
        }
        let hex = core::str::from_utf8(&bytes_hex).ok()?;
        let mut bytes = [0u8; 16];
        for (index, slot) in bytes.iter_mut().enumerate() {
            let pair = hex.get(index * 2..index * 2 + 2)?;
            *slot = u8::from_str_radix(pair, 16).ok()?;
        }
        Some(Self(bytes))
    }
}

impl Serialize for Uuid {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hyphenated())
    }
}

impl<'de> Deserialize<'de> for Uuid {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse_hyphenated(&raw)
            .ok_or_else(|| serde::de::Error::custom("expected a hyphenated UUID"))
    }
}

/// Stands in for a real instant type. Milliseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timestamp(pub i64);

/// A clock reading passed into the machine rather than read by it, which is
/// what makes replay exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LogicalInstant(pub i64);

/// Stands in for a blake3 digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(pub [u8; 32]);

macro_rules! id_newtype {
    ($($name:ident),* $(,)?) => {
        $(
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
            pub struct $name(pub Uuid);
        )*
    };
}

id_newtype!(
    OrgId,
    ProductId,
    FileId,
    MappingId,
    AttemptId,
    JobId,
    ConnectionId,
    CanonicalTermId,
    UserId,
);

/// A marketplace as an account and a login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Marketplace {
    Tes,
    Etsy,
    Tpt,
}

impl Marketplace {
    /// The closed set, in a stable order, for the vocabulary generator.
    pub const ALL: [Self; 3] = [Self::Tes, Self::Etsy, Self::Tpt];

    /// Which branch of the automation rule this marketplace falls in. Total by
    /// exhaustive match, so a marketplace cannot be added without its branch
    /// being decided for it.
    #[must_use]
    pub const fn transport_class(self) -> TransportClass {
        match self {
            // Neither publishes a seller API, so both are the seller-device
            // branch.
            Self::Tes | Self::Tpt => TransportClass::SellerDevice,
            Self::Etsy => TransportClass::OfficialApi,
        }
    }
}

/// Which branch of the automation rule a marketplace falls in, decided by
/// whether the marketplace sanctions the automation. Where it publishes an
/// official API and issues a token for the purpose, automation runs
/// server-side under that token; where it publishes none, every marketplace
/// request originates on the seller's own device under the seller's own
/// session and the server never composes, signs or issues one.
///
/// The rule is the first non-negotiable in `CLAUDE.md` and decision D1 in
/// `docs/notes/design/vendoo-for-teachers-rethink.md`.
///
/// D1 also requires a test that fails the build if a no-API marketplace gains
/// a server transport, and half of that exists.
///
/// The declaration is gated in four places, so re-pointing a marketplace at
/// the other branch fails whichever one is reached first: the transport-class
/// test in `tam-domain`'s registry, the literal pair pinned in this module's
/// own tests, `marketplace_inventory.transport_class` from migration 0043,
/// and the two tests in `tam-storage`'s lease suite that hold that column and
/// this function to each other so a SQL statement cannot route around the
/// Rust.
///
/// What is owed is the other half: nothing checks that no server binary holds
/// a live transport for a `SellerDevice` marketplace. Two do today, each from
/// a cookie jar rather than through a broker — `tam-canary` for Tes and
/// `tam-import` for Tpt — so the check is booked against their dispositions
/// rather than written now, as open question 9 of
/// `docs/notes/design/engine-driver-split.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransportClass {
    /// The marketplace publishes an official API and issues a token for the
    /// purpose, so automation runs server-side on infrastructure we operate.
    OfficialApi,
    /// The marketplace publishes no official API, so every request originates
    /// on the seller's own device under the seller's own session, and the
    /// server is a control plane sending declarative intent instead.
    SellerDevice,
}

impl TransportClass {
    /// The closed set, in a stable order.
    pub const ALL: [Self; 2] = [Self::OfficialApi, Self::SellerDevice];
}

/// The inventory a listing is actually created in, which is the unit the model
/// keys on. Tes runs disjoint GB and US inventories under one marketplace, so
/// keying projections on `Marketplace` would make the entire first chargeable
/// product unrepresentable. Whether one author login reaches both inventories
/// is an assumption rather than a research finding, and it is settled by the
/// M-1 probe on the founder's own account; `Connection` scope depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InventoryId {
    TesGb,
    TesUs,
    TesNz,
    Etsy,
    Tpt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Currency {
    Gbp,
    Usd,
}

impl Currency {
    /// The ISO 4217 code, which is what a marketplace renders and what an
    /// import records as the denomination it read. Distinct from the storage
    /// encoding in `tam-storage`, which is a column value and not a wire one.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Gbp => "GBP",
            Self::Usd => "USD",
        }
    }
}

/// How an inventory decides the currency a price is denominated in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CurrencyRule {
    /// Fixed by the inventory itself. Measured on Tes: fetching two US-inventory
    /// resources from a New Zealand client with `geoCurrency=AUD` cookies still
    /// returned USD offers, so currency follows the inventory rather than the
    /// viewer. The GB-cookie case specifically has not been tested.
    Fixed(Currency),
    /// Set by the seller at shop level. Unverified for Etsy; must be
    /// established before that connector is built.
    SellerScoped,
    /// Not yet probed. `ProjectionBlocked::CurrencyUnknown` is the designed
    /// consumer: a projection into an unmeasured inventory blocks rather than
    /// assuming a currency, and the variant is removed by a probe, not a guess.
    Unmeasured,
}

impl InventoryId {
    /// The closed set, in a stable order; the vocabulary generator and the
    /// closed-set tests read this single source.
    pub const ALL: [Self; 5] = [Self::TesGb, Self::TesUs, Self::TesNz, Self::Etsy, Self::Tpt];

    #[must_use]
    pub const fn marketplace(self) -> Marketplace {
        match self {
            Self::TesGb | Self::TesUs | Self::TesNz => Marketplace::Tes,
            Self::Etsy => Marketplace::Etsy,
            Self::Tpt => Marketplace::Tpt,
        }
    }

    #[must_use]
    pub const fn currency_rule(self) -> CurrencyRule {
        match self {
            Self::TesGb => CurrencyRule::Fixed(Currency::Gbp),
            Self::TesUs => CurrencyRule::Fixed(Currency::Usd),
            // Observed by the founder on their own account: the NZ upload flow
            // carries no currency control and prices denominate in GBP, so the
            // inventory fixes the currency to the account's GBP rather than
            // offering a per-listing choice. Recorded in decisions.md
            // ("The NZ currency, fixed to GBP by observation, 2026-08-28").
            // Re-open if a non-GBP seller's NZ inventory ever shows otherwise.
            #[expect(
                clippy::match_same_arms,
                reason = "the GBP value coincides with TesGb today, but the NZ rule is a re-openable founder observation rather than the definitional GB fact, so the arms stay distinct to carry their own provenance"
            )]
            Self::TesNz => CurrencyRule::Fixed(Currency::Gbp),
            // Confirmed by the founder in their own TPT seller account on
            // 2026-08-29: the marketplace sells in USD and offers no other
            // currency to select. The read's bare `$` is therefore a fact of
            // how TPT renders money and not evidence of which currency it is.
            #[expect(
                clippy::match_same_arms,
                reason = "the USD value coincides with TesUs, but this is a founder observation of one marketplace's pricing rather than that inventory's definitional fact, so the arms stay distinct to carry their own provenance"
            )]
            Self::Tpt => CurrencyRule::Fixed(Currency::Usd),
            Self::Etsy => CurrencyRule::SellerScoped,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MoneyRaw")]
pub struct Money {
    minor_units: i64,
    currency: Currency,
}

/// Deserialisation routes through [`Money::new`], so a wire value cannot
/// bypass the positivity invariant.
#[derive(Deserialize)]
struct MoneyRaw {
    minor_units: i64,
    currency: Currency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoneyError {
    NotPositive,
}

impl core::fmt::Display for MoneyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotPositive => f.write_str("a price is a positive amount of minor units"),
        }
    }
}

impl core::error::Error for MoneyError {}

impl TryFrom<MoneyRaw> for Money {
    type Error = MoneyError;

    fn try_from(raw: MoneyRaw) -> Result<Self, Self::Error> {
        Self::new(raw.minor_units, raw.currency)
    }
}

impl Money {
    /// A positive amount. Zero is not a price; it is the `Free` variant of
    /// `PriceIntent`, which the marketplace parity rules treat differently.
    pub fn new(minor_units: i64, currency: Currency) -> Result<Self, MoneyError> {
        if minor_units <= 0 {
            return Err(MoneyError::NotPositive);
        }
        Ok(Self {
            minor_units,
            currency,
        })
    }

    #[must_use]
    pub const fn minor_units(self) -> i64 {
        self.minor_units
    }

    #[must_use]
    pub const fn currency(self) -> Currency {
        self.currency
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriceIntent {
    Free,
    Paid(Money),
}

/// How one inventory's price is derived. Separate from the parity invariant,
/// which is a cross-inventory check rather than a derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriceRule {
    /// Convert the canonical price at a rate recorded on the mapping.
    Converted {
        rate_micros: i64,
        rounding: Rounding,
    },
    /// The seller set this inventory's price by hand.
    Explicit(PriceIntent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rounding {
    Nearest,
    UpToCharm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileRole {
    Payload,
    Preview,
    Cover,
}

impl FileRole {
    /// The closed set, in a stable order, for the same reader.
    pub const ALL: [Self; 3] = [Self::Payload, Self::Preview, Self::Cover];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    Pdf,
    Pptx,
    Docx,
    Zip,
    Image,
}

impl FileKind {
    /// The closed set, in a stable order; the API's own spelling of these is
    /// what an upload handle and a product view both carry, and the
    /// vocabulary generator reads this single source.
    pub const ALL: [Self; 5] = [Self::Pdf, Self::Pptx, Self::Docx, Self::Zip, Self::Image];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanOutcome {
    Pending,
    Clean { at: Timestamp },
    Infected { signature: String },
    Failed { code: FailureCode },
}

/// What one device saw when it fetched a file we do not hold.
///
/// Every field is the device's word. The instant is the device's own reading
/// rather than ours, which is the rule migration 0045 set for a
/// device-originated row: the seller's assertion and our receipt are two
/// facts, and the receipt is stamped where the row is written rather than
/// carried here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    /// Which of the seller's machines saw it.
    pub device: String,
    /// The digest of the bytes handed onward after the unwrap decision — the
    /// sole entry where a bundle reduces to one file, the bundle otherwise —
    /// and never the container. A marketplace that re-zips a bundle with
    /// different timestamps changes the container's digest while the entry's
    /// is unchanged, so digesting the container would stall a single-file
    /// resource on a mismatch that means nothing.
    pub hash: ContentHash,
    pub byte_len: u64,
    /// The scan the device ran, recorded as the device's and never restated
    /// as a verdict of ours, because we did not see the bytes.
    pub scan: ScanOutcome,
    pub observed_at: Timestamp,
}

/// Where a product file's bytes are, and therefore who vouches for them.
///
/// The two arms are the whole of what integrity means for a file. `Held` is
/// bytes in our object store, whose digest the server computed itself. Under
/// D27 a migrated file's bytes never reach us: they are fetched from the
/// marketplace holding them, on the seller's own device, under the seller's
/// own session, and uploaded to the target in the same run. So `Sourced` names
/// the resource instead and carries what a device reported about it.
///
/// One value rather than a nullable pair because the states are exclusive and
/// the database says so: `product_file_blob_or_source` admits a row that is
/// blob-backed or marketplace-sourced and neither both nor neither. Expressing
/// that here makes the rejected row unconstructible rather than merely
/// rejected, which is the difference between learning at compile time and
/// learning from a constraint violation in a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileBytes {
    Held {
        hash: ContentHash,
        byte_len: u64,
        scan: ScanOutcome,
    },
    Sourced {
        marketplace: Marketplace,
        /// Which of the seller's connections can fetch it, so a revoked
        /// connection is visibly the reason a file became unreachable.
        connection: ConnectionId,
        /// How that marketplace addresses the resource, in the spelling its
        /// own adapter takes.
        resource: String,
        /// Which file inside the resource, where the marketplace hands over a
        /// bundle. `None` is the bundle whole, and its presence is what says
        /// an unwrap happened.
        entry: Option<String>,
        /// The name and type of the bytes handed onward after the unwrap
        /// decision: the entry's own where `entry` is present, the bundle's
        /// and `application/zip` where it is absent. Never the wrapper's name
        /// against the entry's bytes, which is how a seller's worksheet
        /// reaches their own storefront named as a zip.
        payload_file_name: String,
        payload_content_type: String,
        observed: Observation,
    },
}

impl FileBytes {
    /// The digest of this file's bytes, whoever vouched for it.
    ///
    /// For the callers that only need to name the content — deduplication, a
    /// diff, a log line — and never for one deciding whether to trust it. A
    /// caller that cares which of the two it has must match, because that
    /// distinction is the entire reason these are two arms.
    #[must_use]
    pub const fn digest(&self) -> ContentHash {
        match self {
            Self::Held { hash, .. } => *hash,
            Self::Sourced { observed, .. } => observed.hash,
        }
    }

    #[must_use]
    pub const fn byte_len(&self) -> u64 {
        match self {
            Self::Held { byte_len, .. } => *byte_len,
            Self::Sourced { observed, .. } => observed.byte_len,
        }
    }

    /// What is known about this file being clean.
    ///
    /// A reference rather than a copy so the caller sees the whole outcome,
    /// and deliberately not distinguishing whose scan it was: the publish gate
    /// asks only whether a file has been found clean, and Q-b decided the
    /// device's answer is acceptable and advisory. A caller wanting to know
    /// who scanned it matches on the arm.
    #[must_use]
    pub const fn scan(&self) -> &ScanOutcome {
        match self {
            Self::Held { scan, .. } => scan,
            Self::Sourced { observed, .. } => &observed.scan,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductFile {
    pub id: FileId,
    pub role: FileRole,
    pub kind: FileKind,
    pub bytes: FileBytes,
}

/// A product with no payload cannot be listed anywhere, so the empty case is
/// removed rather than validated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadSet {
    head: ProductFile,
    tail: Vec<ProductFile>,
}

impl PayloadSet {
    #[must_use]
    pub fn new(head: ProductFile, tail: Vec<ProductFile>) -> Self {
        Self { head, tail }
    }

    pub fn iter(&self) -> impl Iterator<Item = &ProductFile> {
        std::iter::once(&self.head).chain(self.tail.iter())
    }
}

/// A title as the seller authored it. Per-inventory caps and character rules
/// are applied at projection time, never at authoring time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Title(pub String);

/// How a marketplace counts a title against its cap. Unverified on TPT, whose
/// 80-character cap was established from a sample containing no astral-plane
/// characters and therefore cannot distinguish these cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LengthUnit {
    Bytes,
    Utf16CodeUnits,
    Codepoints,
    GraphemeClusters,
}

impl LengthUnit {
    /// The closed set, in a stable order; the vocabulary generator reads this
    /// single source, because a cap crosses to the client carrying its unit.
    pub const ALL: [Self; 4] = [
        Self::Bytes,
        Self::Utf16CodeUnits,
        Self::Codepoints,
        Self::GraphemeClusters,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingCopy {
    pub body: String,
    /// How the body is written, declared by whatever read or authored it.
    /// Durable rather than derived, because the only alternative is sniffing
    /// the bytes and the tree already ruled that out.
    pub format: CopyFormat,
}

/// How a listing body is written. Declared rather than sniffed: guessing a
/// body's format from its bytes is how a listing acquires escaped markup
/// nobody asked for, which `write_model.rs` already ruled out on the TPT
/// side. A projection whose source and target disagree renders on this
/// declaration — Tes takes either format and posts the matching
/// `descriptionRawType`, TPT takes HTML and the projection renders Markdown
/// into it — so the carriage is a decision about a declared format and never
/// a guess at an undeclared one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CopyFormat {
    Markdown,
    Html,
}

impl CopyFormat {
    /// The closed set, in a stable order, for the same reader.
    pub const ALL: [Self; 2] = [Self::Markdown, Self::Html];
}

/// A source value as an import read yields it, before the relation has been
/// consulted.
///
/// The kind is optional because TPT's 358 facets arrive in one flat namespace
/// and which axis a slug answers is a fact of the seeded relation, not of the
/// array it came out of. Forcing an unclassified slug to some existing kind
/// would be worse than holding it kind-less: a value mislabelled `Topic` is
/// *projected* as a topic rather than named as a loss, which defeats the
/// purpose of keeping it at all.
///
/// `VocabularyPath` stays the typed-axis form and is recovered from one of
/// these only once a kind is known, which is the pure upgrade with no data
/// migration the model promises.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedTerm {
    pub inventory: InventoryId,
    pub kind: Option<TermKind>,
    pub segments: Vec<String>,
    pub native_id: Option<String>,
}

/// The price as the source stated it, before any currency is claimed.
///
/// The denomination is the source's own marker and may be a symbol rather
/// than a code: TPT renders a bare `$` on a New Zealand store, and a
/// `symbol == "$" => Usd` rule would put an unmeasured currency inside
/// `Money`, where it is indistinguishable from a measured one and the parity
/// and price-floor guards then compare across the wrong denomination.
/// Resolution happens only where `CurrencyRule::Fixed` states the answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportedPrice {
    Free,
    Paid {
        minor_units: i64,
        denomination: String,
    },
}

/// The axes an equivalence relates. Here rather than in `tam-domain` because
/// the adapter seam names one too: an imported term crosses in
/// `tam-marketplace`, and `tam-domain` depends on that crate rather than the
/// reverse, so the shared axis label has to sit in the crate both depend on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TermKind {
    Subject,
    Topic,
    ResourceType,
    Phase,
    /// A rights grant. An axis like the others in the relation and unlike
    /// them in one respect the registry carries rather than this enum: no
    /// opt-in delegates it to a computation.
    Licence,
}

impl TermKind {
    /// Every axis, so the client vocabulary is generated from the enum rather
    /// than transcribed beside it.
    pub const ALL: [Self; 5] = [
        Self::Subject,
        Self::Topic,
        Self::ResourceType,
        Self::Phase,
        Self::Licence,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FieldKey {
    Title,
    Description,
    Price,
    Taxonomy,
    Grades,
    Files,
}

/// How many values one canonical field takes. A fact of the canonical model
/// rather than of any inventory, which is why it hangs off `FieldKey` here
/// and not off the per-inventory registry, where `Cardinality` records what
/// one platform's own wire field accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    One,
    Many,
}

impl FieldKey {
    #[must_use]
    pub const fn cardinality(self) -> Arity {
        match self {
            Self::Title | Self::Description | Self::Price => Arity::One,
            Self::Taxonomy | Self::Grades | Self::Files => Arity::Many,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMismatch {
    pub field: FieldKey,
    pub class: MismatchClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MismatchClass {
    /// Entity re-encoding, Unicode normalisation, curly quotes, whitespace
    /// collapse, tag reordering. Expected, and not a defect.
    Normalised,
    Truncated {
        limit_observed: usize,
    },
    Missing,
    WrongField {
        observed_in: FieldKey,
    },
    Unexpected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MismatchResponse {
    Accept,
    Degrade,
    HaltInventory,
    HaltAndPage,
}

impl MismatchClass {
    /// The response is a total function of the class rather than a runbook
    /// paragraph, so an agent editing the classifier cannot leave a class
    /// without an action.
    #[must_use]
    pub const fn response(&self) -> MismatchResponse {
        match self {
            Self::Normalised => MismatchResponse::Accept,
            Self::Truncated { .. } => MismatchResponse::Degrade,
            Self::Missing => MismatchResponse::HaltInventory,
            Self::WrongField { .. } | Self::Unexpected => MismatchResponse::HaltAndPage,
        }
    }
}

/// Closed, versioned and low-cardinality, with an explicit other-case, so a
/// failure list of two hundred items is filterable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FailureCode {
    SelectorNotFound,
    SelectorAmbiguous,
    SelectorResolvedViaFallback,
    PreconditionElementAbsent,
    NavigationCancelled,
    UnexpectedOrigin,
    SubmitNoConfirmation,
    ChallengePresented,
    SessionExpired,
    UploadRejected,
    RateLimited,
    VerificationMismatch,
    FormSchemaDrift,
    /// The loaded selector pack's declared adapter version is not one this
    /// build accepts. Replaces the client-fleet-era `PackExpired`/`PackRejected`.
    AdapterVersionRejected,
    Other,
}

impl FailureCode {
    /// The closed set shared with the client, in a stable order; the
    /// cross-layer test and the vocabulary generator read this single source.
    pub const ALL: [Self; 15] = [
        Self::SelectorNotFound,
        Self::SelectorAmbiguous,
        Self::SelectorResolvedViaFallback,
        Self::PreconditionElementAbsent,
        Self::NavigationCancelled,
        Self::UnexpectedOrigin,
        Self::SubmitNoConfirmation,
        Self::ChallengePresented,
        Self::SessionExpired,
        Self::UploadRejected,
        Self::RateLimited,
        Self::VerificationMismatch,
        Self::FormSchemaDrift,
        Self::AdapterVersionRejected,
        Self::Other,
    ];
}

/// Adapter-supplied free text accompanying a `FailureCode`. Never parsed, never
/// crosswalked to copy, and never permitted to decide an outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureDetail(pub String);

/// The UUIDv5 namespace for idempotency-key derivation, generated once and
/// never rotated, because rotating it would re-key every item in flight.
pub const NAMESPACE_TAM_INTENT: Uuid = Uuid([
    0x1d, 0xe8, 0xf6, 0xc1, 0x9f, 0x8b, 0x4a, 0x55, 0x9b, 0x52, 0x7a, 0x1c, 0x3d, 0x1f, 0x5a, 0x10,
]);

/// The UUIDv5 namespace for canonical taxonomy term ids, generated once and
/// never rotated, because rotating it would orphan every stored term and edge.
/// Deterministic ids are what make reseeding idempotent and make independent
/// GB and NZ crawls converge on the same canonical term.
pub const NAMESPACE_TAM_TAXONOMY: Uuid = Uuid([
    0x7b, 0x3a, 0x22, 0x9e, 0x5d, 0x41, 0x4c, 0x8a, 0x8f, 0x0d, 0x6e, 0x2b, 0x91, 0x54, 0xc7, 0x33,
]);

/// Why a bind folded into an attempt settle left the landing unrecorded.
/// Each variant carries what acting on it needs: the identifier the mapping
/// already holds, the mapping that claims the landed listing, or the binding
/// state the fenced bind was refused against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindAnomaly {
    /// The mapping is bound, and to a different listing than this write
    /// landed on.
    DivergentLanding { existing_remote: String },
    /// Another mapping in the same inventory already claims the landed
    /// listing.
    ClaimedElsewhere { claiming_mapping: MappingId },
    /// The prior-state fence matched no row, in this binding state.
    Refused { binding_state: String },
    /// A removal took a listing down, and the mapping that asked for it is
    /// bound to a different one. Distinct from `Refused`, whose meaning is
    /// that the fence matched no row in this binding state: a sever that
    /// diverges fails on the remote identity while the row is still bound.
    SeverDiverged { existing_remote: String },
    /// A removal severed the binding on a run whose lease had already been
    /// stolen and whose item the stealer had already settled. The attempt
    /// settle is fenced on the attempt row's own epoch, which is written and
    /// compared from the same `LeaseRef` and so cannot mismatch within a
    /// run, while `expire_and_steal` bumps `job_item.lease_epoch` — so the
    /// sever reaches the mapping and no other row records that it did.
    SeveredAfterSteal { lease_epoch: i64 },
}

/// The body carried beside each `job_event.kind`. The serde tag of each
/// variant is exactly one `JobEventKind` name — the agreement test below is
/// the tripwire — and the pair is one tagged union split across the two
/// columns. Bodies stay lean here; the client-facing contract for them is
/// M1h's to version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobEventPayload {
    JobQueued {
        items: u32,
    },
    JobStarted {
        items: u32,
    },
    ItemQueued {
        mapping: MappingId,
    },
    ItemLeased {
        worker: String,
        lease_epoch: i64,
    },
    ItemActionStarted {
        sequence: u32,
        label: String,
    },
    ItemActionFinished {
        sequence: u32,
    },
    ItemBlocked {
        cause: String,
    },
    ItemParked {
        expires_ms: i64,
    },
    /// The park stayed, and what it waits on changed.
    ///
    /// Distinct from [`Self::ItemParked`], which says a park began and when it
    /// ends. This one says neither: the item was already parked and stays
    /// parked, and what moved is the gate — from one a clock was going to
    /// clear to one only the seller can. A second `ItemParked` would put an
    /// expiry in the timeline that had already elapsed and name no gate at
    /// all, which is a worse record than none.
    ItemGateChanged {
        gate: String,
    },
    ItemResumed,
    ItemSettled {
        outcome: String,
    },
    JobSettled {
        succeeded: u32,
        degraded: u32,
        failed: u32,
        ambiguous: u32,
        skipped: u32,
        blocked: u32,
    },
    JobHalted {
        scope: String,
    },
    /// One import run's reconciliation-drain measurement, whole rather than
    /// per row. The kill gate's share is `items_new` over the canonical terms
    /// the run projected, which is `terms_covered + items_new +
    /// items_already_open`; a term already carrying an open item is in that
    /// denominator because it was still a term this run had to translate.
    /// `terms_unmapped` is the falsifier: a native id with no inbound edge
    /// never becomes a canonical term, so it leaves the share untouched, and
    /// a share that falls while this rises is an ingest gap rather than a
    /// converging crosswalk. Both inventories travel in the body because the
    /// stream carries the payload without its job row.
    ImportDrainMeasured {
        source: InventoryId,
        target: InventoryId,
        rows: u32,
        terms_seen: u32,
        terms_unmapped: u32,
        terms_covered: u32,
        items_new: u32,
        items_already_open: u32,
    },
    /// A write landed on a listing the mapping does not record. The clean
    /// dispositions stay silent by construction — a bind, an idempotent
    /// re-land and a verdict with nothing to bind leave nothing to act on —
    /// so a row here is always a live listing the ledger is the only witness
    /// to.
    ItemBindAnomaly {
        anomaly: BindAnomaly,
    },
    /// One page of a device's catalogue import landed.
    ///
    /// The console refetches the request on any event of the organisation's
    /// stream and runs no poller of its own, so an import that emitted nothing
    /// until it finished would show a seller nothing at all while their shop of
    /// several hundred resources was walked. This is what makes progress
    /// visible, and it carries the counts rather than a percentage because the
    /// total is not known until the enumeration ends.
    ///
    /// Anchored to an inert job minted with the request's first page. A
    /// `job_event` row requires a job and an import has none until it
    /// completes; an itemless job is the same device `ImportDrainMeasured`
    /// already uses, and it cannot become a publish because a `queued` item is
    /// what the lease scan claims and it has none.
    ImportPageApplied {
        request: Uuid,
        described: u32,
        skipped: u32,
    },
    /// A device's catalogue import completed, and the write jobs it earned
    /// exist.
    ///
    /// Separate from the page event rather than a flag on it, because the two
    /// are different states to a reader: one is progress and this is the
    /// terminal transition that makes `create_job` readable. `create_job` is
    /// `None` where the catalogue held nothing to publish, which is a
    /// completion rather than a failure. The counts are the request's totals
    /// rather than the last page's.
    ImportCompleted {
        request: Uuid,
        create_job: Option<JobId>,
        described: u32,
        skipped: u32,
    },
}

impl JobEventPayload {
    /// Every kind name, in a stable order, for the vocabulary generator and
    /// the client's stream subscriptions.
    pub const ALL_KINDS: [&'static str; 17] = [
        "JobQueued",
        "JobStarted",
        "ItemQueued",
        "ItemLeased",
        "ItemActionStarted",
        "ItemActionFinished",
        "ItemBlocked",
        "ItemParked",
        "ItemGateChanged",
        "ItemResumed",
        "ItemSettled",
        "JobSettled",
        "JobHalted",
        "ImportDrainMeasured",
        "ItemBindAnomaly",
        "ImportPageApplied",
        "ImportCompleted",
    ];

    /// The serde tag, which is the `job_event.kind` column value. Total, so
    /// adding a variant without a kind string fails to compile here.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::JobQueued { .. } => "JobQueued",
            Self::JobStarted { .. } => "JobStarted",
            Self::ItemQueued { .. } => "ItemQueued",
            Self::ItemLeased { .. } => "ItemLeased",
            Self::ItemActionStarted { .. } => "ItemActionStarted",
            Self::ItemActionFinished { .. } => "ItemActionFinished",
            Self::ItemBlocked { .. } => "ItemBlocked",
            Self::ItemParked { .. } => "ItemParked",
            Self::ItemGateChanged { .. } => "ItemGateChanged",
            Self::ItemResumed => "ItemResumed",
            Self::ItemSettled { .. } => "ItemSettled",
            Self::JobSettled { .. } => "JobSettled",
            Self::JobHalted { .. } => "JobHalted",
            Self::ImportDrainMeasured { .. } => "ImportDrainMeasured",
            Self::ItemBindAnomaly { .. } => "ItemBindAnomaly",
            Self::ImportPageApplied { .. } => "ImportPageApplied",
            Self::ImportCompleted { .. } => "ImportCompleted",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Currency, FailureCode, Marketplace, Money, TransportClass};

    /// The closed set shared with the client. The single or-pattern arm carries
    /// no wildcard, so adding or removing a variant fails compilation here and
    /// forces the cross-layer contract to be revisited deliberately.
    #[test]
    fn failure_code_is_a_closed_set_of_fifteen() {
        for code in FailureCode::ALL {
            match code {
                FailureCode::SelectorNotFound
                | FailureCode::SelectorAmbiguous
                | FailureCode::SelectorResolvedViaFallback
                | FailureCode::PreconditionElementAbsent
                | FailureCode::NavigationCancelled
                | FailureCode::UnexpectedOrigin
                | FailureCode::SubmitNoConfirmation
                | FailureCode::ChallengePresented
                | FailureCode::SessionExpired
                | FailureCode::UploadRejected
                | FailureCode::RateLimited
                | FailureCode::VerificationMismatch
                | FailureCode::FormSchemaDrift
                | FailureCode::AdapterVersionRejected
                | FailureCode::Other => {}
            }
        }
        assert_eq!(
            FailureCode::ALL.len(),
            15,
            "FailureCode is a closed cross-layer contract; a change must update the client alongside this count"
        );
    }

    /// Total over `Marketplace::ALL` by a wildcard-free match, so a
    /// marketplace added without a decided branch fails compilation here as
    /// well as at the declaration. The expected value is a literal rather
    /// than a second call, so a flipped branch fails rather than agreeing
    /// with itself.
    #[test]
    fn every_marketplace_declares_the_branch_the_automation_rule_puts_it_in() {
        for marketplace in Marketplace::ALL {
            let expected = match marketplace {
                // Neither publishes a seller API, so every request must
                // originate on the seller's own device.
                Marketplace::Tes | Marketplace::Tpt => TransportClass::SellerDevice,
                Marketplace::Etsy => TransportClass::OfficialApi,
            };
            assert_eq!(
                marketplace.transport_class(),
                expected,
                "{marketplace:?} must declare the branch the automation rule puts it in"
            );
        }
    }

    /// The closed set shared with the client. The wildcard-free match means a
    /// variant added or removed fails compilation here, and the literal wire
    /// spellings pin the tokens a client branches on.
    #[test]
    fn the_transport_class_is_a_closed_pair_with_a_stable_wire_spelling() {
        for class in TransportClass::ALL {
            let wire = match class {
                TransportClass::OfficialApi => "\"OfficialApi\"",
                TransportClass::SellerDevice => "\"SellerDevice\"",
            };
            assert_eq!(
                serde_json::to_string(&class).expect("TransportClass serialises"),
                wire,
                "the wire spelling is a cross-layer contract and must not drift"
            );
            let back: TransportClass = serde_json::from_str(wire)
                .expect("TransportClass deserialises from its own output");
            assert_eq!(
                back, class,
                "the class must survive a serde round trip unchanged"
            );
        }
        assert_eq!(
            TransportClass::ALL.len(),
            2,
            "the automation rule has exactly two branches"
        );
    }

    #[test]
    fn money_survives_a_serde_round_trip() {
        let money = Money::new(150, Currency::Gbp).expect("150 minor units is a valid price");
        let json = serde_json::to_string(&money).expect("Money serialises");
        let back: Money =
            serde_json::from_str(&json).expect("Money deserialises from its own output");
        assert_eq!(
            back, money,
            "Money must survive a serde round trip unchanged"
        );
    }

    /// The fourteen serde tags and the fourteen kind strings are one set; the
    /// wildcard-free construction plus this agreement loop is the tripwire.
    #[test]
    fn every_job_event_tag_is_its_kind_string() {
        let samples = [
            super::JobEventPayload::JobQueued { items: 1 },
            super::JobEventPayload::JobStarted { items: 1 },
            super::JobEventPayload::ItemQueued {
                mapping: super::MappingId(super::Uuid([1; 16])),
            },
            super::JobEventPayload::ItemLeased {
                worker: "w".to_owned(),
                lease_epoch: 1,
            },
            super::JobEventPayload::ItemActionStarted {
                sequence: 1,
                label: "submit".to_owned(),
            },
            super::JobEventPayload::ItemActionFinished { sequence: 1 },
            super::JobEventPayload::ItemBlocked {
                cause: "reauth".to_owned(),
            },
            super::JobEventPayload::ItemParked { expires_ms: 1 },
            super::JobEventPayload::ItemResumed,
            super::JobEventPayload::ItemSettled {
                outcome: "succeeded".to_owned(),
            },
            super::JobEventPayload::JobSettled {
                succeeded: 1,
                degraded: 0,
                failed: 0,
                ambiguous: 0,
                skipped: 0,
                blocked: 0,
            },
            super::JobEventPayload::JobHalted {
                scope: "org_inventory".to_owned(),
            },
            super::JobEventPayload::ImportDrainMeasured {
                source: super::InventoryId::TesGb,
                target: super::InventoryId::TesNz,
                rows: 1,
                terms_seen: 3,
                terms_unmapped: 1,
                terms_covered: 1,
                items_new: 1,
                items_already_open: 0,
            },
            super::JobEventPayload::ItemBindAnomaly {
                anomaly: super::BindAnomaly::Refused {
                    binding_state: "severed".to_owned(),
                },
            },
            // Absent since the variant was added: the assertion below counted
            // a hand-maintained literal rather than the vocabulary, so it read
            // fourteen against fifteen kinds and certified a property it did
            // not have. Counting `ALL_KINDS` is what stops that recurring.
            super::JobEventPayload::ItemGateChanged {
                gate: "taxonomy".to_owned(),
            },
            super::JobEventPayload::ImportPageApplied {
                request: super::Uuid([0x71; 16]),
                described: 25,
                skipped: 1,
            },
            super::JobEventPayload::ImportCompleted {
                request: super::Uuid([0x71; 16]),
                create_job: Some(super::JobId(super::Uuid([0x72; 16]))),
                described: 120,
                skipped: 3,
            },
        ];
        assert_eq!(
            samples.len(),
            super::JobEventPayload::ALL_KINDS.len(),
            "one sample per JobEventKind, counted against the vocabulary rather than a literal \
             that drifts from it"
        );
        for payload in samples {
            let encoded = serde_json::to_value(&payload).expect("a payload serialises");
            let tag = encoded
                .as_object()
                .and_then(|object| object.keys().next().cloned())
                .unwrap_or_else(|| encoded.as_str().unwrap_or_default().to_owned());
            assert_eq!(
                tag,
                payload.kind(),
                "the serde tag and the kind string must agree"
            );
        }
    }

    #[test]
    fn money_rejects_a_non_positive_amount_on_the_wire() {
        let refused = serde_json::from_str::<Money>(r#"{"minor_units":0,"currency":"Gbp"}"#);
        assert!(
            refused.is_err(),
            "zero minor units must not deserialise into Money; zero is PriceIntent::Free"
        );
    }
}
