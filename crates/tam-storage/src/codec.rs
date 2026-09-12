//! Total conversions between domain values and their column renderings.
//!
//! Every decode is defensive even where a CHECK constraint guards the column,
//! because a decode error names corruption precisely instead of panicking on
//! it. `FailureCode` alone stores its serde name verbatim: schema.md requires
//! the column and the type to share one name across the worker, the API, the
//! client and the operator dashboard.

use chrono::{DateTime, Utc};
use tam_domain::ItemOperation;
use tam_marketplace::{LifecycleTransition, ListingState, RemoteListingId};
use tam_types::{
    ContentHash, Currency, FailureCode, FileKind, FileRole, InventoryId, Money, PriceIntent,
    ScanOutcome, Timestamp,
};

use crate::StorageError;

pub(crate) fn uuid_to_db(id: tam_types::Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

pub(crate) fn uuid_from_db(id: uuid::Uuid) -> tam_types::Uuid {
    tam_types::Uuid(id.into_bytes())
}

pub(crate) fn timestamp_to_db(at: Timestamp) -> Result<DateTime<Utc>, StorageError> {
    DateTime::<Utc>::from_timestamp_millis(at.0)
        .ok_or(StorageError::TimestampOutOfRange { millis: at.0 })
}

pub(crate) fn timestamp_from_db(at: DateTime<Utc>) -> Timestamp {
    Timestamp(at.timestamp_millis())
}

pub(crate) fn hash_to_db(hash: ContentHash) -> Vec<u8> {
    hash.0.to_vec()
}

pub(crate) fn hash_from_db(bytes: &[u8]) -> Result<ContentHash, StorageError> {
    <[u8; 32]>::try_from(bytes)
        .map(ContentHash)
        .map_err(|_| StorageError::CorruptRow {
            reason: format!("content hash of {} bytes, expected 32", bytes.len()),
        })
}

pub(crate) fn hash_hex(hash: ContentHash) -> String {
    use core::fmt::Write;
    let mut hex = String::with_capacity(64);
    for byte in hash.0 {
        // infallible on String; the Result is the trait's, not the writer's
        let _unused: core::fmt::Result = write!(hex, "{byte:02x}");
    }
    hex
}

pub(crate) const PRICE_KIND_FREE: &str = "free";
pub(crate) const PRICE_KIND_PAID: &str = "paid";
const CURRENCY_GBP: &str = "gbp";
const CURRENCY_USD: &str = "usd";

/// The column rendering of a denomination, for the columns that hold a
/// currency beside an amount without a `price_kind` to go with it: an import
/// run item's price is what a read said, and a read that said nothing is an
/// absent amount rather than a `free` kind.
pub(crate) const fn currency_to_db(currency: Currency) -> &'static str {
    match currency {
        Currency::Gbp => CURRENCY_GBP,
        Currency::Usd => CURRENCY_USD,
    }
}

pub(crate) fn currency_from_db(raw: &str) -> Result<Currency, StorageError> {
    match raw {
        c if c == CURRENCY_GBP => Ok(Currency::Gbp),
        c if c == CURRENCY_USD => Ok(Currency::Usd),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown currency {other:?}"),
        }),
    }
}

pub(crate) struct PriceColumns {
    pub(crate) kind: &'static str,
    pub(crate) minor_units: Option<i64>,
    pub(crate) currency: Option<&'static str>,
}

impl PriceColumns {
    pub(crate) fn from_intent(price: PriceIntent) -> Self {
        match price {
            PriceIntent::Free => Self {
                kind: PRICE_KIND_FREE,
                minor_units: None,
                currency: None,
            },
            PriceIntent::Paid(money) => Self {
                kind: PRICE_KIND_PAID,
                minor_units: Some(money.minor_units()),
                currency: Some(match money.currency() {
                    Currency::Gbp => CURRENCY_GBP,
                    Currency::Usd => CURRENCY_USD,
                }),
            },
        }
    }
}

pub(crate) fn price_from_db(
    kind: &str,
    minor_units: Option<i64>,
    currency: Option<String>,
) -> Result<PriceIntent, StorageError> {
    match (kind, minor_units, currency) {
        (k, None, None) if k == PRICE_KIND_FREE => Ok(PriceIntent::Free),
        (k, Some(units), Some(cur)) if k == PRICE_KIND_PAID => {
            let currency = match cur.as_str() {
                c if c == CURRENCY_GBP => Currency::Gbp,
                c if c == CURRENCY_USD => Currency::Usd,
                other => {
                    return Err(StorageError::CorruptRow {
                        reason: format!("unknown currency {other:?}"),
                    })
                }
            };
            Money::new(units, currency)
                .map(PriceIntent::Paid)
                .map_err(|_| StorageError::CorruptRow {
                    reason: format!("non-positive paid amount {units}"),
                })
        }
        (k, units, cur) => Err(StorageError::CorruptRow {
            reason: format!("inconsistent price columns ({k:?}, {units:?}, {cur:?})"),
        }),
    }
}

pub(crate) const fn file_role_to_db(role: FileRole) -> &'static str {
    match role {
        FileRole::Payload => "payload",
        FileRole::Preview => "preview",
        FileRole::Cover => "cover",
    }
}

pub(crate) fn file_role_from_db(raw: &str) -> Result<FileRole, StorageError> {
    match raw {
        "payload" => Ok(FileRole::Payload),
        "preview" => Ok(FileRole::Preview),
        "cover" => Ok(FileRole::Cover),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown file role {other:?}"),
        }),
    }
}

pub(crate) const fn file_kind_to_db(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Pdf => "pdf",
        FileKind::Pptx => "pptx",
        FileKind::Docx => "docx",
        FileKind::Zip => "zip",
        FileKind::Image => "image",
    }
}

pub(crate) fn file_kind_from_db(raw: &str) -> Result<FileKind, StorageError> {
    match raw {
        "pdf" => Ok(FileKind::Pdf),
        "pptx" => Ok(FileKind::Pptx),
        "docx" => Ok(FileKind::Docx),
        "zip" => Ok(FileKind::Zip),
        "image" => Ok(FileKind::Image),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown file kind {other:?}"),
        }),
    }
}

pub(crate) struct ScanColumns {
    pub(crate) state: &'static str,
    pub(crate) signature: Option<String>,
    pub(crate) scanned_at: Option<DateTime<Utc>>,
    pub(crate) failure_code: Option<&'static str>,
}

impl ScanColumns {
    pub(crate) fn from_outcome(scan: &ScanOutcome) -> Result<Self, StorageError> {
        Ok(match scan {
            ScanOutcome::Pending => Self {
                state: "pending",
                signature: None,
                scanned_at: None,
                failure_code: None,
            },
            ScanOutcome::Clean { at } => Self {
                state: "clean",
                signature: None,
                scanned_at: Some(timestamp_to_db(*at)?),
                failure_code: None,
            },
            ScanOutcome::Infected { signature } => Self {
                state: "infected",
                signature: Some(signature.clone()),
                scanned_at: None,
                failure_code: None,
            },
            ScanOutcome::Failed { code } => Self {
                state: "failed",
                signature: None,
                scanned_at: None,
                failure_code: Some(failure_code_to_db(*code)),
            },
        })
    }
}

pub(crate) fn scan_from_db(
    state: &str,
    signature: Option<String>,
    scanned_at: Option<DateTime<Utc>>,
    failure_code: Option<&str>,
) -> Result<ScanOutcome, StorageError> {
    match (state, signature, scanned_at, failure_code) {
        ("pending", None, None, None) => Ok(ScanOutcome::Pending),
        ("clean", None, Some(at), None) => Ok(ScanOutcome::Clean {
            at: timestamp_from_db(at),
        }),
        ("infected", Some(signature), _, None) => Ok(ScanOutcome::Infected { signature }),
        ("failed", None, _, Some(code)) => Ok(ScanOutcome::Failed {
            code: failure_code_from_db(code)?,
        }),
        (state, signature, scanned_at, failure_code) => Err(StorageError::CorruptRow {
            reason: format!(
                "inconsistent scan columns ({state:?}, {signature:?}, {scanned_at:?}, {failure_code:?})"
            ),
        }),
    }
}

/// The serde name verbatim, per schema.md: one name across every layer.
pub(crate) const fn failure_code_to_db(code: FailureCode) -> &'static str {
    match code {
        FailureCode::SelectorNotFound => "SelectorNotFound",
        FailureCode::SelectorAmbiguous => "SelectorAmbiguous",
        FailureCode::SelectorResolvedViaFallback => "SelectorResolvedViaFallback",
        FailureCode::PreconditionElementAbsent => "PreconditionElementAbsent",
        FailureCode::NavigationCancelled => "NavigationCancelled",
        FailureCode::UnexpectedOrigin => "UnexpectedOrigin",
        FailureCode::SubmitNoConfirmation => "SubmitNoConfirmation",
        FailureCode::ChallengePresented => "ChallengePresented",
        FailureCode::SessionExpired => "SessionExpired",
        FailureCode::UploadRejected => "UploadRejected",
        FailureCode::RateLimited => "RateLimited",
        FailureCode::VerificationMismatch => "VerificationMismatch",
        FailureCode::FormSchemaDrift => "FormSchemaDrift",
        FailureCode::AdapterVersionRejected => "AdapterVersionRejected",
        FailureCode::Other => "Other",
    }
}

pub(crate) fn failure_code_from_db(raw: &str) -> Result<FailureCode, StorageError> {
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
    ALL.into_iter()
        .find(|code| failure_code_to_db(*code) == raw)
        .ok_or_else(|| StorageError::CorruptRow {
            reason: format!("unknown failure code {raw:?}"),
        })
}

/// The three-column rendering of a `RemoteListingId`, shared by the mapping's
/// binding and the settled `write_attempt`: the `*_remote_id_shape` CHECK
/// constraints require the url and numeric columns to be exclusive per kind.
pub(crate) struct RemoteIdColumns<'a> {
    pub(crate) kind: &'static str,
    pub(crate) url: Option<&'a str>,
    pub(crate) numeric_id: Option<i64>,
}

impl<'a> RemoteIdColumns<'a> {
    pub(crate) fn encode(id: &'a RemoteListingId) -> Result<Self, StorageError> {
        Ok(match id {
            RemoteListingId::Tes { url } => Self {
                kind: "tes",
                url: Some(url.as_str()),
                numeric_id: None,
            },
            RemoteListingId::Tpt { product_id } => Self {
                kind: "tpt",
                url: None,
                numeric_id: Some(i64::try_from(*product_id).map_err(|_| {
                    StorageError::Inconsistent {
                        reason: format!("tpt product id {product_id} exceeds the column range"),
                    }
                })?),
            },
            RemoteListingId::Etsy { listing_id } => Self {
                kind: "etsy",
                url: None,
                numeric_id: Some(i64::try_from(*listing_id).map_err(|_| {
                    StorageError::Inconsistent {
                        reason: format!("etsy listing id {listing_id} exceeds the column range"),
                    }
                })?),
            },
        })
    }
}

pub(crate) const fn listing_state_to_db(state: ListingState) -> &'static str {
    match state {
        ListingState::Draft => "draft",
        ListingState::Live => "live",
    }
}

pub(crate) fn listing_state_from_db(raw: &str) -> Result<ListingState, StorageError> {
    match raw {
        "draft" => Ok(ListingState::Draft),
        "live" => Ok(ListingState::Live),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown listing state {other:?}"),
        }),
    }
}

/// The six-column rendering of an [`ItemOperation`]: what the item does, the
/// listing it asserts it does it to, and the transition it states. The
/// subject reuses [`RemoteIdColumns`], because an item's subject and a
/// mapping's binding are the same shape and `job_item_subject_shape` is
/// `mapping_remote_id_shape` restated.
pub(crate) struct OperationColumns<'a> {
    pub(crate) operation: &'static str,
    pub(crate) subject_kind: Option<&'static str>,
    pub(crate) subject_url: Option<&'a str>,
    pub(crate) subject_numeric_id: Option<i64>,
    pub(crate) state_from: Option<&'static str>,
    pub(crate) state_to: Option<&'static str>,
}

impl<'a> OperationColumns<'a> {
    pub(crate) fn encode(operation: &'a ItemOperation) -> Result<Self, StorageError> {
        Ok(match operation {
            ItemOperation::Create => Self {
                operation: "create",
                subject_kind: None,
                subject_url: None,
                subject_numeric_id: None,
                state_from: None,
                state_to: None,
            },
            ItemOperation::Revise {
                subject,
                transition,
            } => {
                let remote = RemoteIdColumns::encode(subject)?;
                Self {
                    operation: "revise",
                    subject_kind: Some(remote.kind),
                    subject_url: remote.url,
                    subject_numeric_id: remote.numeric_id,
                    state_from: Some(listing_state_to_db(transition.from)),
                    state_to: Some(listing_state_to_db(transition.to)),
                }
            }
            ItemOperation::Publish { to } => Self {
                operation: "publish",
                subject_kind: None,
                subject_url: None,
                subject_numeric_id: None,
                state_from: None,
                state_to: Some(listing_state_to_db(*to)),
            },
            ItemOperation::Remove { subject, state } => {
                let remote = RemoteIdColumns::encode(subject)?;
                Self {
                    operation: "remove",
                    subject_kind: Some(remote.kind),
                    subject_url: remote.url,
                    subject_numeric_id: remote.numeric_id,
                    state_from: Some(listing_state_to_db(*state)),
                    state_to: None,
                }
            }
        })
    }
}

/// The same six columns as read back, before they are decoded. A struct
/// rather than six arguments, which is also what lets a query hand its row
/// over field by field without an ordering to get wrong.
pub(crate) struct StoredOperation {
    pub(crate) operation: String,
    pub(crate) subject_kind: Option<String>,
    pub(crate) subject_url: Option<String>,
    pub(crate) subject_numeric_id: Option<i64>,
    pub(crate) state_from: Option<String>,
    pub(crate) state_to: Option<String>,
}

impl StoredOperation {
    pub(crate) fn decode(self) -> Result<ItemOperation, StorageError> {
        let subject = match self.subject_kind {
            Some(kind) => Some(crate::mapping::remote_id_from_db(
                &kind,
                self.subject_url,
                self.subject_numeric_id,
            )?),
            None => None,
        };
        let from = self
            .state_from
            .as_deref()
            .map(listing_state_from_db)
            .transpose()?;
        let to = self
            .state_to
            .as_deref()
            .map(listing_state_from_db)
            .transpose()?;
        match (self.operation.as_str(), subject, from, to) {
            ("create", None, None, None) => Ok(ItemOperation::Create),
            ("publish", None, None, Some(to)) => Ok(ItemOperation::Publish { to }),
            ("revise", Some(subject), Some(from), Some(to)) => Ok(ItemOperation::Revise {
                subject,
                transition: LifecycleTransition { from, to },
            }),
            ("remove", Some(subject), Some(state), None) => {
                Ok(ItemOperation::Remove { subject, state })
            }
            (operation, _, _, _) => Err(StorageError::CorruptRow {
                reason: format!("inconsistent operation columns for {operation:?}"),
            }),
        }
    }
}

pub(crate) const fn marketplace_to_db(marketplace: tam_types::Marketplace) -> &'static str {
    match marketplace {
        tam_types::Marketplace::Tes => "tes",
        tam_types::Marketplace::Etsy => "etsy",
        tam_types::Marketplace::Tpt => "tpt",
    }
}

pub(crate) const fn inventory_to_db(inventory: InventoryId) -> &'static str {
    match inventory {
        InventoryId::Tes => "tes",
        InventoryId::Etsy => "etsy",
        InventoryId::Tpt => "tpt",
    }
}

pub(crate) fn inventory_from_db(raw: &str) -> Result<InventoryId, StorageError> {
    match raw {
        "tes" => Ok(InventoryId::Tes),
        "etsy" => Ok(InventoryId::Etsy),
        "tpt" => Ok(InventoryId::Tpt),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown inventory {other:?}"),
        }),
    }
}

pub(crate) const fn copy_format_to_db(format: tam_types::CopyFormat) -> &'static str {
    match format {
        tam_types::CopyFormat::Markdown => "markdown",
        tam_types::CopyFormat::Html => "html",
    }
}

pub(crate) fn copy_format_from_db(raw: &str) -> Result<tam_types::CopyFormat, StorageError> {
    match raw {
        "markdown" => Ok(tam_types::CopyFormat::Markdown),
        "html" => Ok(tam_types::CopyFormat::Html),
        other => Err(StorageError::Inconsistent {
            reason: format!("unknown body format {other}"),
        }),
    }
}

pub(crate) const fn term_kind_to_db(kind: tam_domain::TermKind) -> &'static str {
    match kind {
        tam_domain::TermKind::Subject => "subject",
        tam_domain::TermKind::Topic => "topic",
        tam_domain::TermKind::ResourceType => "resource_type",
        tam_domain::TermKind::Phase => "phase",
        tam_domain::TermKind::Licence => "licence",
    }
}

pub(crate) fn term_kind_from_db(raw: &str) -> Result<tam_domain::TermKind, StorageError> {
    match raw {
        "subject" => Ok(tam_domain::TermKind::Subject),
        "topic" => Ok(tam_domain::TermKind::Topic),
        "resource_type" => Ok(tam_domain::TermKind::ResourceType),
        "phase" => Ok(tam_domain::TermKind::Phase),
        "licence" => Ok(tam_domain::TermKind::Licence),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown term kind {other:?}"),
        }),
    }
}

pub(crate) const fn edge_kind_to_db(kind: tam_domain::EdgeKind) -> &'static str {
    match kind {
        tam_domain::EdgeKind::Exact => "exact",
        tam_domain::EdgeKind::Broader => "broader",
        tam_domain::EdgeKind::Narrower => "narrower",
    }
}

pub(crate) fn edge_kind_from_db(raw: &str) -> Result<tam_domain::EdgeKind, StorageError> {
    match raw {
        "exact" => Ok(tam_domain::EdgeKind::Exact),
        "broader" => Ok(tam_domain::EdgeKind::Broader),
        "narrower" => Ok(tam_domain::EdgeKind::Narrower),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown edge kind {other:?}"),
        }),
    }
}

/// The four decider columns as one value: `(decided_by, decided_source,
/// decided_user, decided_org)`. The migration's CHECK makes the split total.
pub(crate) type DeciderColumns = (
    &'static str,
    Option<String>,
    Option<uuid::Uuid>,
    Option<uuid::Uuid>,
);

pub(crate) fn decider_to_db(decider: &tam_domain::Decider) -> DeciderColumns {
    match decider {
        tam_domain::Decider::Imported { source } => ("imported", Some(source.clone()), None, None),
        tam_domain::Decider::Human { user, org } => (
            "human",
            None,
            Some(uuid_to_db(user.0)),
            Some(uuid_to_db(org.0)),
        ),
    }
}

pub(crate) fn decider_from_db(
    decided_by: &str,
    source: Option<String>,
    user: Option<uuid::Uuid>,
    org: Option<uuid::Uuid>,
) -> Result<tam_domain::Decider, StorageError> {
    match (decided_by, source, user, org) {
        ("imported", Some(source), None, None) => Ok(tam_domain::Decider::Imported { source }),
        ("human", None, Some(user), Some(org)) => Ok(tam_domain::Decider::Human {
            user: tam_types::UserId(uuid_from_db(user)),
            org: tam_types::OrgId(uuid_from_db(org)),
        }),
        (other, _, _, _) => Err(StorageError::CorruptRow {
            reason: format!("decider columns violate totality for {other:?}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{failure_code_to_db, hash_hex};
    use tam_types::{ContentHash, FailureCode};

    /// schema.md: the failure-code column and the type share one name across
    /// every layer, and the serde tag is that name.
    #[test]
    fn failure_code_column_text_is_the_serde_name() {
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
            let serde_name = serde_json::to_value(code).expect("a failure code serialises");
            assert_eq!(
                serde_name.as_str(),
                Some(failure_code_to_db(code)),
                "column text and serde tag must agree for {code:?}"
            );
        }
    }

    #[test]
    fn hash_hex_is_lowercase_and_sixty_four_chars() {
        let hex = hash_hex(ContentHash([0xAB; 32]));
        assert_eq!(hex.len(), 64, "a 32-byte digest renders as 64 nibbles");
        assert!(hex.starts_with("abab"), "nibbles render lowercase: {hex}");
    }
}
