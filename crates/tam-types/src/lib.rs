//! The pure vocabulary and wire ADTs shared by every layer of the workspace.
//!
//! Every definition is promoted verbatim from `docs/design/sketches/domain.rs`,
//! the artefact of record for the domain types. The crate carries no I/O
//! dependency: `serde` only, and deserialisation routes through the smart
//! constructor wherever a type carries an invariant, so a wire value cannot
//! reach a state the constructor would refuse.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Stands in for the `uuid` crate's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Uuid(pub [u8; 16]);

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
}

impl InventoryId {
    #[must_use]
    pub const fn marketplace(self) -> Marketplace {
        match self {
            Self::TesGb | Self::TesUs => Marketplace::Tes,
            Self::Etsy => Marketplace::Etsy,
            Self::Tpt => Marketplace::Tpt,
        }
    }

    #[must_use]
    pub const fn currency_rule(self) -> CurrencyRule {
        match self {
            Self::TesGb => CurrencyRule::Fixed(Currency::Gbp),
            Self::TesUs => CurrencyRule::Fixed(Currency::Usd),
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

/// Adapter-supplied free text accompanying a `FailureCode`. Never parsed, never
/// crosswalked to copy, and never permitted to decide an outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureDetail(pub String);

#[cfg(test)]
mod tests {
    use super::{Currency, FailureCode, Money};

    /// The closed set shared with the client. The single or-pattern arm carries
    /// no wildcard, so adding or removing a variant fails compilation here and
    /// forces the cross-layer contract to be revisited deliberately.
    #[test]
    fn failure_code_is_a_closed_set_of_fifteen() {
        const ALL: [FailureCode; 15] = [
            FailureCode::SelectorNotFound,
            FailureCode::SelectorAmbiguous,
            FailureCode::SelectorResolvedViaFallback,
            FailureCode::PreconditionElementAbsent,
            FailureCode::NavigationCancelled,
            FailureCode::UnexpectedOrigin,
            FailureCode::SubmitNoConfirmation,
            FailureCode::ChallengePresented,
            FailureCode::SessionExpired,
            FailureCode::UploadRejected,
            FailureCode::RateLimited,
            FailureCode::VerificationMismatch,
            FailureCode::FormSchemaDrift,
            FailureCode::AdapterVersionRejected,
            FailureCode::Other,
        ];
        for code in ALL {
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
            ALL.len(),
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

    #[test]
    fn money_rejects_a_non_positive_amount_on_the_wire() {
        let refused = serde_json::from_str::<Money>(r#"{"minor_units":0,"currency":"Gbp"}"#);
        assert!(
            refused.is_err(),
            "zero minor units must not deserialise into Money; zero is PriceIntent::Free"
        );
    }
}
