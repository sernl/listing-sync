//! The pure vocabulary and wire ADTs shared by every layer of the workspace.
//!
//! Every definition is promoted verbatim from `docs/design/sketches/domain.rs`,
//! the artefact of record for the domain types. The crate carries no I/O
//! dependency: `serde` only, and deserialisation routes through the smart
//! constructor wherever a type carries an invariant, so a wire value cannot
//! reach a state the constructor would refuse.

#![forbid(unsafe_code)]

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

/// How an inventory decides the currency a price is denominated in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CurrencyRule {
    /// Fixed by the inventory itself. Measured on Tes: fetching two US-inventory
    /// resources from a New Zealand client with `geoCurrency=AUD` cookies still
    /// returned USD offers, so currency follows the inventory rather than the
    /// viewer. The GB-cookie case specifically has not been tested.
    Fixed(Currency),
    /// Set by the seller at shop level. Unverified for Etsy and TPT; must be
    /// established before either connector is built.
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
            Self::Etsy | Self::Tpt => CurrencyRule::SellerScoped,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    Pdf,
    Pptx,
    Docx,
    Zip,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanOutcome {
    Pending,
    Clean { at: Timestamp },
    Infected { signature: String },
    Failed { code: FailureCode },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductFile {
    pub id: FileId,
    pub role: FileRole,
    pub kind: FileKind,
    pub hash: ContentHash,
    pub byte_len: u64,
    pub scan: ScanOutcome,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingCopy {
    pub body: String,
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
}

impl JobEventPayload {
    /// Every kind name, in a stable order, for the vocabulary generator and
    /// the client's stream subscriptions.
    pub const ALL_KINDS: [&'static str; 14] = [
        "JobQueued",
        "JobStarted",
        "ItemQueued",
        "ItemLeased",
        "ItemActionStarted",
        "ItemActionFinished",
        "ItemBlocked",
        "ItemParked",
        "ItemResumed",
        "ItemSettled",
        "JobSettled",
        "JobHalted",
        "ImportDrainMeasured",
        "ItemBindAnomaly",
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
            Self::ItemResumed => "ItemResumed",
            Self::ItemSettled { .. } => "ItemSettled",
            Self::JobSettled { .. } => "JobSettled",
            Self::JobHalted { .. } => "JobHalted",
            Self::ImportDrainMeasured { .. } => "ImportDrainMeasured",
            Self::ItemBindAnomaly { .. } => "ItemBindAnomaly",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Currency, FailureCode, Money};

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
        ];
        assert_eq!(samples.len(), 14, "one sample per JobEventKind");
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
