//! The TPT read model: the shapes the two services answer with, and the
//! fallible parsers that turn them into typed rows.
//!
//! Kept apart from the request builders because parsing is where TPT's wire
//! quirks live — money as display text, an identifier that is a number in one
//! response and a string in another, two date formats on one object — and
//! each of them is a way a naive read goes quietly wrong.

use serde_json::Value;
use tam_marketplace::RemoteListingId;

/// A product's durable identifier. TPT serialises it as a numeric string on
/// the product read and accepts it as an unquoted integer on the analytics
/// read; both are the same identifier and this type is the only place that
/// coercion is spelt out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProductId(pub u64);

impl ProductId {
    #[must_use]
    pub const fn remote(self) -> RemoteListingId {
        RemoteListingId::Tpt { product_id: self.0 }
    }
}

/// The import addresses a catalogue row by its numeric id, and TPT's own
/// handle is unsigned, so the conversion is fallible rather than lossy: a
/// negative id is a corrupt manifest row and says so.
impl TryFrom<i64> for ProductId {
    type Error = core::num::TryFromIntError;

    fn try_from(id: i64) -> Result<Self, Self::Error> {
        u64::try_from(id).map(Self)
    }
}

impl core::fmt::Display for ProductId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The alias every statistics query selects under, so one parser serves both
/// root fields. The captured client aliases one metric per selection; this
/// adapter asks for one metric per request and names it the same way each time.
pub const STATS_ALIAS: &str = "totals";

/// A money field as TPT renders it: the symbol exactly as the wire wrote it,
/// and the amount in hundredths. The currency is deliberately not resolved to
/// a [`tam_types::Currency`] here — the wire renders a bare `$` and the
/// create form's minimum price carries no currency at all, so this parse has
/// nothing to resolve from. Which currency that dollar is denominates from
/// the inventory's own rule at the adapter boundary, where the founder's
/// 2026-08-29 account check settled it as USD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TptPrice {
    pub symbol: String,
    pub minor_units: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriceParseError {
    NoDigits(String),
    Unexpected(String),
    TooManyFractionalDigits(String),
    OutOfRange(String),
}

impl core::fmt::Display for PriceParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoDigits(text) => write!(f, "price {text:?} carries no digits"),
            Self::Unexpected(text) => write!(f, "price {text:?} carries an unexpected character"),
            Self::TooManyFractionalDigits(text) => {
                write!(f, "price {text:?} has more than two fractional digits")
            }
            Self::OutOfRange(text) => write!(f, "price {text:?} does not fit in minor units"),
        }
    }
}

impl core::error::Error for PriceParseError {}

/// Two fractional digits, which is what every observed TPT money string
/// renders; a third would change the scale and is refused rather than rounded.
const MINOR_UNITS_PER_UNIT: i64 = 100;

/// Parses a pre-formatted money string such as `"$10.40"`. TPT returns every
/// money field as display text and never as a number, so an unparsed price
/// would be compared as a string and `"$9.00"` would sort above `"$10.40"`.
/// Fallible throughout: a malformed value fails the read rather than the
/// process.
pub fn parse_display_price(display: &str) -> Result<TptPrice, PriceParseError> {
    let text = display.trim();
    let mut symbol = String::new();
    let mut units = String::new();
    let mut fraction: Option<String> = None;
    for character in text.chars() {
        let started = !units.is_empty() || fraction.is_some();
        match character {
            '0'..='9' => match fraction.as_mut() {
                Some(digits) => digits.push(character),
                None => units.push(character),
            },
            ',' if started => {}
            '.' if started && fraction.is_none() => fraction = Some(String::new()),
            _ if !started && character != '-' && !character.is_whitespace() => {
                symbol.push(character);
            }
            _ => return Err(PriceParseError::Unexpected(text.to_owned())),
        }
    }
    if units.is_empty() {
        return Err(PriceParseError::NoDigits(text.to_owned()));
    }
    let whole: i64 = units
        .parse()
        .map_err(|_| PriceParseError::OutOfRange(text.to_owned()))?;
    let digits = fraction.unwrap_or_default();
    if digits.chars().count() > 2 {
        return Err(PriceParseError::TooManyFractionalDigits(text.to_owned()));
    }
    let mut padded = digits;
    while padded.chars().count() < 2 {
        padded.push('0');
    }
    let hundredths: i64 = padded
        .parse()
        .map_err(|_| PriceParseError::OutOfRange(text.to_owned()))?;
    let minor_units = whole
        .checked_mul(MINOR_UNITS_PER_UNIT)
        .and_then(|scaled| scaled.checked_add(hundredths))
        .ok_or_else(|| PriceParseError::OutOfRange(text.to_owned()))?;
    Ok(TptPrice {
        symbol,
        minor_units,
    })
}

/// A shelf the seller owns, as the product read names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TptCategory {
    pub id: String,
    pub name: String,
}

/// One row of the seller's own catalogue in TPT's vocabulary, carrying what
/// reconciliation needs and nothing it does not: no thumbnails, no Easel
/// properties, no bundle tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TptCatalogueEntry {
    pub id: ProductId,
    pub name: String,
    pub canonical_slug: String,
    pub kind: Option<String>,
    pub item_type: Option<String>,
    pub status: Option<String>,
    pub price: TptPrice,
    pub is_free: bool,
    pub is_featured: bool,
    /// One flat namespace: grade, subject, resource type and file format all
    /// arrive as undifferentiated slugs in this one array.
    pub taxonomy_tags: Vec<String>,
    pub categories: Vec<TptCategory>,
    /// Carried verbatim. TPT renders `lastModifiedAt` as `2025-01-19T02:01:33`
    /// and `postDate` as `2025-01-15 05:33:11` on the same object, so no
    /// single parse serves both and this adapter interprets neither.
    pub last_modified_at: Option<String>,
}

impl TptCatalogueEntry {
    #[must_use]
    pub const fn remote(&self) -> RemoteListingId {
        self.id.remote()
    }
}

/// One page of the walk, with the counts the walk terminates against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CataloguePage {
    pub entries: Vec<TptCatalogueEntry>,
    pub total_results: u64,
    pub current_page: u64,
    pub total_pages: u64,
}

/// One all-time or resolved metric value for one of the seller's products.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceStat {
    pub resource: ProductId,
    /// The wire carries an untyped JSON number for every metric — a count for
    /// sales, an amount for earnings, a ratio for the Easel assign rate — so
    /// it is kept as read rather than reinterpreted as money.
    pub total_value: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeError(pub String);

impl core::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "TPT response shape: {}", self.0)
    }
}

impl core::error::Error for ShapeError {}

/// TPT serialises the same identifier as a number in one place and a string
/// in another — `categories[].id` is `1359903` while the gateway's custom
/// categories give `"1359903"` for the same entity — so an id is read through
/// this rather than through one of the two accessors.
fn scalar_id(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => Some(text.clone()),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

fn optional_str(row: &Value, field: &str) -> Option<String> {
    row.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn slug_list(row: &Value, field: &str) -> Vec<String> {
    row.get(field)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("id").and_then(scalar_id))
                .collect()
        })
        .unwrap_or_default()
}

fn categories(row: &Value) -> Vec<TptCategory> {
    row.get("categories")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(TptCategory {
                        id: entry.get("id").and_then(scalar_id)?,
                        name: optional_str(entry, "name").unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parses one product row. A row without a numeric id or with an unparseable
/// price fails the page rather than being dropped, so a shape change cannot
/// shorten a catalogue silently.
fn parse_product(row: &Value) -> Result<TptCatalogueEntry, ShapeError> {
    let raw_id = row
        .get("id")
        .and_then(scalar_id)
        .ok_or_else(|| ShapeError("a product row carries no id".to_owned()))?;
    let id = raw_id
        .parse::<u64>()
        .map_err(|_| ShapeError(format!("product id {raw_id:?} is not numeric")))
        .map(ProductId)?;
    let price = row
        .get("price")
        .and_then(Value::as_str)
        .ok_or_else(|| ShapeError(format!("product {id} carries no price string")))
        .and_then(|display| {
            parse_display_price(display).map_err(|error| ShapeError(error.to_string()))
        })?;
    Ok(TptCatalogueEntry {
        id,
        name: optional_str(row, "name").unwrap_or_default(),
        canonical_slug: optional_str(row, "canonicalSlug").unwrap_or_default(),
        kind: optional_str(row, "kind"),
        item_type: optional_str(row, "itemType"),
        status: optional_str(row, "status"),
        price,
        is_free: row.get("isFree").and_then(Value::as_bool).unwrap_or(false),
        is_featured: row
            .get("isFeatured")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        taxonomy_tags: slug_list(row, "taxonomyTags"),
        categories: categories(row),
        last_modified_at: optional_str(row, "lastModifiedAt"),
    })
}

/// One product as the edit form's own read yields it. Wider than a catalogue
/// row on the fields canonicalisation needs — the description above all — and
/// narrower on the ones only reconciliation does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadPageProduct {
    pub id: ProductId,
    pub name: String,
    /// TPT stores and returns the body as HTML. Declared rather than sniffed:
    /// `write_model`'s `description_html` already records that guessing a
    /// body's format from its bytes is how a listing acquires escaped markup
    /// nobody asked for.
    pub description: String,
    pub is_free: bool,
    pub price: TptPrice,
    /// The draft line as the wire states it, or `None` where the read did not
    /// carry it. The captures on file predate the `status` selection, so
    /// `None` is their honest answer rather than a defaulted `ACTIVE`.
    pub status: Option<String>,
    /// One flat namespace: grade, subject, resource type, audience and file
    /// format all arrive as undifferentiated slugs here, and which axis a
    /// slug answers is a fact of the seeded relation rather than of this
    /// array.
    pub taxonomy_tags: Vec<String>,
    /// Seller-owned shelves rather than a platform vocabulary.
    pub categories: Vec<TptCategory>,
    pub item_type: Option<String>,
    pub copyright_declaration: Option<String>,
    /// `localization.countryIdFlag`, the state of TPT's own "Appropriate for
    /// {country}" checkbox.
    ///
    /// Read back because an edit is a full replace: posting a constant here
    /// clears a box the seller ticked, and this is the only place the current
    /// state can be learned. The country beside it is not read; the flag is
    /// the whole field on the wire and the country only labels it.
    ///
    /// `None` is a response that carried no `localization` object, which the
    /// older captures do, and is not the same as a measured false.
    pub appropriate_for_country: Option<bool>,
}

/// Parses the edit form's own product read. The id is not in the response —
/// the query addresses one product by id and the row carries no `id` field —
/// so the caller's own id is threaded through rather than invented.
pub fn parse_upload_page_product(
    body: &Value,
    id: ProductId,
) -> Result<UploadPageProduct, ShapeError> {
    let row = body
        .pointer("/data/products/0")
        .ok_or_else(|| ShapeError(format!("no product row for {id}")))?;
    let is_free = row.get("isFree").and_then(Value::as_bool).unwrap_or(false);
    let price = row
        .get("price")
        .and_then(Value::as_str)
        .ok_or_else(|| ShapeError(format!("product {id} carries no price string")))
        .and_then(|display| {
            parse_display_price(display).map_err(|error| ShapeError(error.to_string()))
        })?;
    Ok(UploadPageProduct {
        id,
        name: optional_str(row, "name").unwrap_or_default(),
        description: optional_str(row, "description").unwrap_or_default(),
        is_free,
        price,
        status: optional_str(row, "status"),
        taxonomy_tags: slug_list(row, "taxonomyTags"),
        categories: categories(row),
        item_type: optional_str(row, "itemType"),
        copyright_declaration: optional_str(row, "copyrightDeclaration"),
        appropriate_for_country: row
            .pointer("/localization/countryIdFlag")
            .and_then(Value::as_bool),
    })
}

fn count(page_info: &Value, field: &str) -> Result<u64, ShapeError> {
    page_info
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ShapeError(format!("pageInfo carries no {field}")))
}

/// Parses one `MyProductListings` page: the rows and the counts the walk
/// terminates against.
pub fn parse_catalogue_page(body: &Value) -> Result<CataloguePage, ShapeError> {
    let resources = body
        .pointer("/data/seller/resources")
        .ok_or_else(|| ShapeError("no data.seller.resources in the response".to_owned()))?;
    let rows = resources
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| ShapeError("resources.results is not an array".to_owned()))?;
    let entries = rows
        .iter()
        .map(parse_product)
        .collect::<Result<Vec<_>, ShapeError>>()?;
    let page_info = resources
        .get("pageInfo")
        .ok_or_else(|| ShapeError("resources carries no pageInfo".to_owned()))?;
    Ok(CataloguePage {
        entries,
        total_results: count(page_info, "totalResultsCount")?,
        current_page: count(page_info, "currentPage")?,
        total_pages: count(page_info, "totalPageCount")?,
    })
}

/// Parses the `RemoveResource` answer, which carries the deleted product's
/// own id and nothing else. An answer without one is not a confirmation, and
/// whether it echoes the product the caller asked to delete is the caller's
/// check to make.
pub fn parse_resource_delete(body: &Value) -> Result<ProductId, ShapeError> {
    let raw = body
        .pointer("/data/resourceDelete/id")
        .and_then(scalar_id)
        .ok_or_else(|| ShapeError("no data.resourceDelete.id in the response".to_owned()))?;
    raw.parse::<u64>()
        .map_err(|_| ShapeError(format!("deleted product id {raw:?} is not numeric")))
        .map(ProductId)
}

/// Parses the Relay envelope both statistics root fields answer with. The
/// cursors decode to `rank:N` and are discarded: this adapter batches by
/// resource id rather than walking the ranking.
pub fn parse_stats_edges(body: &Value) -> Result<Vec<ResourceStat>, ShapeError> {
    let edges = body
        .pointer(&format!("/data/{STATS_ALIAS}/edges"))
        .and_then(Value::as_array)
        .ok_or_else(|| ShapeError(format!("no data.{STATS_ALIAS}.edges in the response")))?;
    edges
        .iter()
        .map(|edge| {
            let node = edge
                .get("node")
                .ok_or_else(|| ShapeError("a statistics edge carries no node".to_owned()))?;
            let raw = node
                .get("resourceId")
                .and_then(scalar_id)
                .ok_or_else(|| ShapeError("a statistics node carries no resourceId".to_owned()))?;
            let resource = raw
                .parse::<u64>()
                .map_err(|_| ShapeError(format!("resourceId {raw:?} is not numeric")))
                .map(ProductId)?;
            let total_value = node
                .get("totalValue")
                .and_then(Value::as_f64)
                .ok_or_else(|| ShapeError(format!("resource {resource} carries no totalValue")))?;
            Ok(ResourceStat {
                resource,
                total_value,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_display_price, parse_stats_edges, PriceParseError, ProductId, ShapeError};
    use serde_json::json;
    use tam_marketplace::RemoteListingId;

    #[test]
    fn a_display_price_parses_to_minor_units() {
        for (text, minor) in [
            ("$3.00", 300),
            ("$10.40", 1040),
            ("$0.00", 0),
            ("$21", 2100),
        ] {
            let price = parse_display_price(text).expect("an observed money string parses");
            assert_eq!(price.minor_units, minor, "{text} is {minor} minor units");
            assert_eq!(price.symbol, "$", "the symbol is carried, not interpreted");
        }
    }

    #[test]
    fn a_thousands_separator_does_not_shift_the_scale() {
        let price = parse_display_price("$1,234.56").expect("a grouped amount parses");
        assert_eq!(
            price.minor_units, 123_456,
            "the separator is presentation and must not reach the number"
        );
    }

    #[test]
    fn a_malformed_price_is_an_error_and_never_a_panic() {
        assert!(
            matches!(
                parse_display_price("$1.0.0"),
                Err(PriceParseError::Unexpected(_))
            ),
            "a second decimal point is refused"
        );
        assert!(
            matches!(
                parse_display_price("$1.005"),
                Err(PriceParseError::TooManyFractionalDigits(_))
            ),
            "a third fractional digit changes the scale and is refused rather than rounded"
        );
        assert!(
            matches!(
                parse_display_price("free"),
                Err(PriceParseError::NoDigits(_))
            ),
            "a value carrying no digits is refused"
        );
        assert!(
            matches!(
                parse_display_price("$9223372036854775808.00"),
                Err(PriceParseError::OutOfRange(_))
            ),
            "an amount past i64 is refused rather than wrapped"
        );
    }

    #[test]
    fn a_statistics_node_without_a_total_fails_the_batch() {
        let body = json!({"data": {"totals": {"edges": [
            {"cursor": "cmFuazox", "node": {"resourceId": "1"}}
        ]}}});
        assert!(
            matches!(parse_stats_edges(&body), Err(ShapeError(_))),
            "a metric with no value is a shape change, not a zero"
        );
    }

    #[test]
    fn a_product_id_addresses_the_listing_the_seam_names() {
        assert_eq!(
            ProductId(12_854_712).remote(),
            RemoteListingId::Tpt {
                product_id: 12_854_712
            },
            "the numeric id is the durable identifier; the slug is decorative"
        );
    }
}
