//! The values the seller supplies: what a control holds and what its smart
//! constructor refuses about one value at a time.
//!
//! The closed sets a control chooses *among* live in
//! [`vocabularies`](super::vocabularies) instead, because they are the
//! marketplace's enumerations rather than the seller's input and they carry a
//! wire id this crate must never derive from a position.
//!
//! Refusals that need the whole product, or the capture's measured caps, live
//! in [`validation`](super::validation).

use tam_types::Money;

use crate::product::validation::AuthoringError;
use crate::product::vocabularies::{TaxCode, ThumbnailMode};

// -------------------------------------------------------------- the scalars

/// A product title, at most 80 UTF-16 code units.
///
/// The unit is `maxlength`'s own: `#ItemName` carries a real `maxlength="80"`
/// so the browser truncates at 80 UTF-16 code units. Whether TPT's server
/// counts the same way is unmeasured, so this refuses what the browser would
/// refuse and claims nothing beyond it.
pub const TITLE_MAX_UTF16_UNITS: usize = 80;

/// TPT's own advisory: "Free resources should be 10 pages or fewer." A
/// tooltip on the Free Resource checkbox rather than a validated bound, so
/// exceeding it is a warning the seller sees here rather than a refusal —
/// they would otherwise learn it from TPT, later, after a write.
pub const FREE_RESOURCE_PAGE_GUIDANCE: u32 = 10;

/// The product's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductName(String);

impl ProductName {
    /// Refuses a blank name and one over the form's own cap. Whitespace at
    /// either end is trimmed, because a title that differs from its neighbour
    /// only by a trailing space is one a seller cannot tell apart.
    pub fn new(raw: &str) -> Result<Self, AuthoringError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(AuthoringError::NameMissing);
        }
        let used = trimmed.encode_utf16().count();
        if used > TITLE_MAX_UTF16_UNITS {
            return Err(AuthoringError::NameTooLong {
                used,
                limit: TITLE_MAX_UTF16_UNITS,
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One `data[TaxonomyTags][]` value: a facet slug from
/// `docs/design/data/tpt-vocabulary.json`.
///
/// Membership is not checked here. The catalogue holds 358 slugs and this
/// crate reads no files, so the API resolves a slug against the capture and
/// this type carries the shape only: non-empty, no surrounding whitespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FacetSlug(String);

impl FacetSlug {
    pub fn new(raw: &str) -> Result<Self, AuthoringError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(AuthoringError::EmptySlug);
        }
        Ok(Self(trimmed.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ---------------------------------------------------------------- the files

/// One uploaded file, named by the content hash the upload returned.
///
/// The hash is the whole handle: bytes are content-addressed per tenant, so a
/// hash an organisation never stored resolves to no blob. Kept as text here
/// rather than as `ContentHash` because this model is what a form holds and a
/// form holds what the upload endpoint handed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadRef(String);

impl UploadRef {
    /// A 64-character lowercase hex blake3 digest, refused otherwise.
    pub fn new(raw: &str) -> Result<Self, AuthoringError> {
        let ok = raw.len() == 64
            && raw
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if ok {
            Ok(Self(raw.to_owned()))
        } else {
            Err(AuthoringError::MalformedUploadRef)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The Files group: the payload, its two optional previews, and the thumbnail
/// decision with its four slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileGroup {
    /// `data[ItemDigital][product]`. Required: a product with no payload
    /// cannot be listed anywhere and the catalogue refuses one at commit.
    pub payload: UploadRef,
    /// `data[ItemDigital][preview]`.
    pub preview: Option<UploadRef>,
    /// `data[Upload][videopreview]`.
    pub video_preview: Option<UploadRef>,
    pub thumbnail_mode: ThumbnailMode,
    /// `data[ItemDigital][thumb1..thumb4]`, first slot first. The first is
    /// TPT's "Main Cover"; the rest are optional.
    pub thumbnails: Vec<UploadRef>,
}

// ---------------------------------------------------------------- the price

/// The Price group, split on the branch the form itself is gated on.
///
/// Free and priced are separate variants rather than a price beside a flag,
/// because ticking Free hides Price, Multiple Licenses and Tax Code on TPT's
/// own form: a free listing carrying an amount is a state the form cannot
/// produce, and a type that can represent it is one something will eventually
/// write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriceGroup {
    Free,
    Paid(PaidPrice),
}

/// `data[Item][price]`, `[license_price]`, `[discountprice]` and the tax code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaidPrice {
    price: Money,
    additional_licence: Money,
    bundle_discount: Option<Money>,
    tax_code: TaxCode,
}

impl PaidPrice {
    /// Every part stated. The additional-licence price is carried explicitly
    /// and is never derived here: TPT's help centre says "Although the default
    /// discount is 10% off, you can choose whatever discount seems right to
    /// you", so 90 percent is the form's pre-fill and treating it as a rule
    /// would overwrite a seller's own figure on every sync (D6). The form
    /// seeds the field with [`suggested_additional_licence`]; the projection
    /// never calls it.
    pub fn new(
        price: Money,
        additional_licence: Money,
        bundle_discount: Option<Money>,
        tax_code: TaxCode,
    ) -> Result<Self, AuthoringError> {
        if price.currency() != additional_licence.currency() {
            return Err(AuthoringError::PriceCurrencyMismatch);
        }
        if bundle_discount.is_some_and(|amount| amount.currency() != price.currency()) {
            return Err(AuthoringError::PriceCurrencyMismatch);
        }
        Ok(Self {
            price,
            additional_licence,
            bundle_discount,
            tax_code,
        })
    }

    #[must_use]
    pub const fn price(&self) -> Money {
        self.price
    }

    #[must_use]
    pub const fn additional_licence(&self) -> Money {
        self.additional_licence
    }

    #[must_use]
    pub const fn bundle_discount(&self) -> Option<Money> {
        self.bundle_discount
    }

    #[must_use]
    pub const fn tax_code(&self) -> TaxCode {
        self.tax_code
    }
}

/// TPT's `multiple_license_price_percentage`, which its page bootstrap states
/// as 90.
pub const ADDITIONAL_LICENCE_PERCENTAGE: i64 = 90;

/// The figure the form pre-fills the Multiple Licenses field with, rounded
/// down to the minor unit.
///
/// A pre-fill and nothing else. It exists so the arithmetic has one home
/// rather than being restated in TypeScript, and calling it anywhere in a
/// projection would be the defect D6 names: a seller's own additional-licence
/// price silently overwritten on every sync.
#[must_use]
pub fn suggested_additional_licence(price: Money) -> Option<Money> {
    let minor = price
        .minor_units()
        .checked_mul(ADDITIONAL_LICENCE_PERCENTAGE)?
        .checked_div(100)?;
    Money::new(minor, price.currency()).ok()
}

// ----------------------------------------------------------- the categories

/// The Categories group. Every member is a `data[TaxonomyTags][]` slug except
/// the custom categories, which are the seller's own shelves and reach the
/// wire as `data[Category][Category][]` row ids.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CategoryGroup {
    pub grades: Vec<FacetSlug>,
    pub subject_areas: Vec<FacetSlug>,
    pub tags: Vec<FacetSlug>,
    pub formats: Vec<FacetSlug>,
    /// Seller-scoped shelves rather than a platform vocabulary, so no member
    /// set is held anywhere and no cap is measured.
    pub custom_categories: Vec<String>,
}

/// One alignment the seller claimed.
///
/// The published code and TPT's node id are both carried, and the node id is
/// optional, because they come from different places and go stale differently:
/// the code is the framework owner's and is stable, the node id is TPT's own
/// search-index identifier and is exactly the kind that gets rebuilt. An
/// alignment held with a code and no id is one we can display and cannot yet
/// post, which is a state worth being able to represent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardAlignment {
    pub framework: crate::product::vocabularies::StandardsFramework,
    pub code: String,
    pub tpt_node_id: Option<u64>,
}

/// The Details group. Nothing in it is required, and TPT marks none of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DetailGroup {
    pub teaching_duration: Option<crate::product::vocabularies::TeachingDuration>,
    /// `data[ItemsProperty][pages]`, read back as `filePreview.pageCount`.
    pub pages_or_slides: Option<u32>,
    pub answer_key: Option<crate::product::vocabularies::AnswerKey>,
}

#[cfg(test)]
mod tests {
    use super::{
        suggested_additional_licence, PaidPrice, PriceGroup, ProductName, UploadRef,
        TITLE_MAX_UTF16_UNITS,
    };
    use crate::product::fixtures::{product, HASH};
    use crate::product::validation::AuthoringError;
    use crate::product::vocabularies::TaxCode;
    use tam_types::{Currency, Money};

    /// Free and priced are separate variants, so a free listing carrying an
    /// amount or a tax code is not a value this model can hold.
    #[test]
    fn a_free_listing_cannot_also_carry_a_price() {
        let free = product();
        assert_eq!(
            free.price,
            PriceGroup::Free,
            "ticking Free hides Price, Multiple Licenses and Tax Code on TPT's own form"
        );
        let priced = PaidPrice::new(
            Money::new(450, Currency::Usd).expect("a positive amount"),
            Money::new(405, Currency::Usd).expect("a positive amount"),
            None,
            TaxCode::DigitalBooks,
        )
        .expect("one currency throughout");
        assert_eq!(
            priced.tax_code(),
            TaxCode::DigitalBooks,
            "a priced listing states a tax code, and the type gives it nowhere to be absent"
        );
    }

    #[test]
    fn a_price_and_its_additional_licence_must_be_one_currency() {
        assert_eq!(
            PaidPrice::new(
                Money::new(450, Currency::Usd).expect("a positive amount"),
                Money::new(405, Currency::Gbp).expect("a positive amount"),
                None,
                TaxCode::DigitalBooks,
            ),
            Err(AuthoringError::PriceCurrencyMismatch),
            "two denominations on one listing is a product nobody can price"
        );
    }

    #[test]
    fn a_title_is_refused_blank_and_over_eighty_utf16_units() {
        assert_eq!(ProductName::new("   "), Err(AuthoringError::NameMissing));
        let long = "a".repeat(TITLE_MAX_UTF16_UNITS + 1);
        assert_eq!(
            ProductName::new(&long),
            Err(AuthoringError::NameTooLong {
                used: 81,
                limit: 80
            }),
            "the browser's own `maxlength` is the bound this mirrors"
        );
        assert_eq!(
            ProductName::new(&"a".repeat(TITLE_MAX_UTF16_UNITS)).map(|name| name.as_str().len()),
            Ok(80),
            "and exactly eighty is accepted"
        );
    }

    #[test]
    fn an_upload_reference_is_a_lowercase_hex_digest_and_nothing_else() {
        assert!(UploadRef::new(HASH).is_ok());
        assert_eq!(
            UploadRef::new(&HASH.to_uppercase()),
            Err(AuthoringError::MalformedUploadRef),
            "the upload endpoint hands back lowercase hex, so anything else was not its answer"
        );
        assert_eq!(
            UploadRef::new("cafe"),
            Err(AuthoringError::MalformedUploadRef)
        );
    }

    /// D6: the ninety percent is the form's pre-fill and never the
    /// projection's rule.
    #[test]
    fn the_additional_licence_prefill_is_ninety_percent_rounded_down() {
        let price = Money::new(450, Currency::Usd).expect("a positive amount");
        assert_eq!(
            suggested_additional_licence(price).map(Money::minor_units),
            Some(405)
        );
        let odd = Money::new(99, Currency::Usd).expect("a positive amount");
        assert_eq!(
            suggested_additional_licence(odd).map(Money::minor_units),
            Some(89),
            "rounded down to the minor unit, so the pre-fill never exceeds the discount TPT \
             states"
        );
    }
}
