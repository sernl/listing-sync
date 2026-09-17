//! The catalogue as one spreadsheet: a row per resource, and a status, price
//! and link for every inventory the registry knows.
//!
//! Metadata only. No file, no byte of one and no credential reaches this
//! document; every cell is something the seller already reads on their own
//! console, gathered through the catalogue repository under the same tenant
//! pin the product list uses.
//!
//! Two readings this module had to take, because the founder's sentence
//! ("a CSV of the catalogue with each marketplace's listing status, price and
//! link") is coarser than the model underneath it:
//!
//! A column group is per inventory rather than per marketplace. Tes runs
//! disjoint GB, US and NZ inventories under one marketplace, each with its own
//! listing, its own page and its own price, so one "TES link" cell would have
//! to drop two of three. The group is named the way the console names the same
//! platform, so a seller reads the same word in both places.
//!
//! The marketplace price is the one the seller set by hand, which is the only
//! per-inventory figure this system holds: `PriceRule::Converted` records a
//! rate rather than an amount, nothing in the repository derives an amount
//! from it, and inventing that arithmetic here would put a number in front of
//! a seller that no publishing path agrees with. A converted rule therefore
//! exports an empty price rather than a computed one. The denomination travels
//! in the cell beside the amount because it follows the inventory, so a US
//! listing's figure is not the same money as a GB one.
//!
//! A price the seller has approved for one marketplace is that marketplace's
//! figure, ahead of the mint-time snapshot on the mapping. The two disagree
//! the moment an approval lands — the mapping's price is whatever the product
//! cost when the mapping was minted and no statement ever updates it — and
//! the approved one is what the publishing path freezes and posts, so
//! exporting the other would put a number in front of the seller that their
//! own listing contradicts. A resource with no approval still exports the
//! mapping's snapshot, unchanged: absence of an approval is not an approval
//! of the canonical price, and this document must not read as though it were.
//!
//! The date in the filename is the server's own civil date in UTC, so a seller
//! east of Greenwich exporting in their morning gets a file dated the day
//! before theirs. It is deliberate rather than defaulted: the alternative is
//! naming a file from an offset the client asserts, and nothing else in this
//! API takes a date from a caller. Whether a seller's local date is worth a
//! request parameter is a founder decision, not one to make here.

use axum::extract::{Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use serde::Deserialize;
use tam_domain::registry::listing_url::listing_url;
use tam_storage::{
    ExportedResource, LedgerCursor, ProductRepo, ResourceCollectionRepo, StorageError,
};
use tam_types::{Currency, InventoryId, Money, OrgId, PriceIntent, ProductId, Timestamp, Uuid};

use crate::error::APIError;
use crate::{AppState, OrgContext};

/// How many resources one turn of the page walk reads. The whole document is
/// buffered either way; the page exists so one catalogue is not one statement.
///
/// Public because the walk's only severe test is the one that crosses this
/// boundary, and a test naming its own 200 would stop crossing it the day the
/// number moved.
pub const PAGE: i64 = 200;

/// The columns every row opens with, before the per-inventory groups.
const CATALOGUE_COLUMNS: [&str; 7] = [
    "Resource ID",
    "Title",
    "Price",
    "Currency",
    "Labels",
    "Created",
    "Updated",
];

/// What one inventory's group names, in the order its cells are written.
const LISTING_COLUMNS: [&str; 3] = ["status", "price", "link"];

/// The inventories a group is written for, Tes first and TPT next, which is
/// the order the founder asked the marketplaces in. Held against
/// `InventoryId::ALL` by a test, so an inventory added to the model is a
/// failing test rather than a column that quietly stops being exported.
const EXPORT_ORDER: [InventoryId; 3] = [InventoryId::Tes, InventoryId::Tpt, InventoryId::Etsy];

/// How the console names each inventory on a strip narrow enough to read.
/// Total by exhaustive match, matching `SHORT_NAME` in `web/src/lib/platforms.ts`.
const fn short_name(inventory: InventoryId) -> &'static str {
    match inventory {
        InventoryId::Tes => "TES",
        InventoryId::Etsy => "Etsy",
        InventoryId::Tpt => "TPT",
    }
}

/// Which resources the document covers.
///
/// Absent is the whole catalogue, which is what this route answered before a
/// collection existed and still answers. A collection or an explicit list
/// narrows it, and the narrowing is a filter over the same page walk rather
/// than a second read: the columns are per inventory and built from
/// `export_page`'s own join, so a selection-shaped statement would be a
/// second definition of the document's rows.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExportParams {
    /// A collection whose members are exported, in catalogue order rather
    /// than the collection's own: a spreadsheet is sorted by whoever opens it,
    /// and the row order a CSV carries is not a thing the seller set here.
    #[serde(default)]
    pub collection: Option<String>,
    /// A comma-separated tick list, for a selection the seller has not named.
    #[serde(default)]
    pub products: Option<String>,
}

pub(crate) async fn export_catalogue(
    State(state): State<AppState>,
    context: OrgContext,
    Query(params): Query<ExportParams>,
) -> Result<impl IntoResponse, APIError> {
    let only = selected(&state, context.org, &params).await?;
    let document = document(&state, context.org, only.as_deref()).await?;
    let today = civil_date((state.wall)());
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"teachouse-resources-{today}.csv\""),
            ),
        ],
        document,
    ))
}

/// The resources this request names, or `None` for the whole catalogue.
///
/// A collection this organisation does not hold answers an empty document
/// rather than not-found: the pin is what decides, and a caller guessing
/// identifiers must not learn which guesses were right from the difference.
/// An unparseable identifier in the list is refused, because that is a client
/// fault rather than a stale tab.
async fn selected(
    state: &AppState,
    org: OrgId,
    params: &ExportParams,
) -> Result<Option<Vec<ProductId>>, APIError> {
    if let Some(collection) = params.collection.as_deref() {
        let named = uuid::Uuid::parse_str(collection.trim()).map_err(|_unused| {
            APIError::new(
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                crate::error::APIErrorEntry::new(
                    "the collection parameter is a collection identifier",
                )
                .kind(crate::error::APIErrorKind::Validation),
            )
        })?;
        let members = ResourceCollectionRepo::new(state.pool.clone())
            .members(org, Uuid(*named.as_bytes()))
            .await
            .map_err(|error| storage_fault(state, &error))?;
        return Ok(Some(
            members.into_iter().map(|member| member.product).collect(),
        ));
    }
    let Some(listed) = params.products.as_deref() else {
        return Ok(None);
    };
    listed
        .split(',')
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(|raw| {
            uuid::Uuid::parse_str(raw)
                .map(|parsed| ProductId(Uuid(*parsed.as_bytes())))
                .map_err(|_unused| {
                    APIError::new(
                        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                        crate::error::APIErrorEntry::new(
                            "the products parameter is a comma-separated list of resource \
                             identifiers",
                        )
                        .kind(crate::error::APIErrorKind::Validation),
                    )
                })
        })
        .collect::<Result<Vec<_>, APIError>>()
        .map(Some)
}

fn storage_fault(state: &AppState, error: &StorageError) -> APIError {
    crate::jobs::storage_fault(state, error)
}

/// The document, gathered through the export page walk under this
/// organisation's row-level security.
///
/// Two statements per page and none per resource: `export_page` reads a
/// page's products, their labels and their listings together, because the
/// per-resource composition it replaced cost about eighteen statements and two
/// transactions for every row of the document.
///
/// `only` names the resources a selection admits, and `None` is the whole
/// catalogue. The walk is the same either way and the filter is on the rows,
/// because the page read is what composes a row from four tables and a
/// selection-shaped variant of it would be a second definition of the
/// document.
async fn document(
    state: &AppState,
    org: OrgId,
    only: Option<&[ProductId]>,
) -> Result<String, APIError> {
    let products = ProductRepo::new(state.pool.clone());
    let mut out = String::new();
    write_row(&mut out, &header_row());
    let mut cursor: Option<LedgerCursor> = None;
    loop {
        let page = products
            .export_page(org, cursor, PAGE)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        let Some(last) = page.last() else {
            break;
        };
        cursor = Some(LedgerCursor {
            created_at: last.created_at,
            id: last.id.0,
        });
        let exhausted = i64::try_from(page.len()).unwrap_or(i64::MAX) < PAGE;
        let subjects: Vec<ProductId> = page.iter().map(|resource| resource.id).collect();
        // Per inventory rather than per resource, and per page rather than
        // per document: the figure is a question about one resource on one
        // marketplace, and two marketplaces can hold two different approved
        // prices for the same resource.
        let mut approved: Vec<(InventoryId, Vec<(ProductId, PriceIntent)>)> =
            Vec::with_capacity(EXPORT_ORDER.len());
        for inventory in EXPORT_ORDER {
            approved.push((
                inventory,
                tam_storage::rule_capture::approved_prices(
                    &state.pool,
                    org,
                    &subjects,
                    tam_storage::rule_capture::PricingScope::CrossList(inventory),
                )
                .await
                .map_err(|error| storage_fault(state, &error))?,
            ));
        }
        for resource in page
            .iter()
            .filter(|resource| only.is_none_or(|ids| ids.contains(&resource.id)))
        {
            write_row(&mut out, &row(resource, &approved));
        }
        if exhausted {
            break;
        }
    }
    Ok(out)
}

fn header_row() -> Vec<String> {
    let mut cells: Vec<String> = CATALOGUE_COLUMNS
        .iter()
        .map(|column| (*column).to_owned())
        .collect();
    for inventory in EXPORT_ORDER {
        for column in LISTING_COLUMNS {
            cells.push(format!("{} {column}", short_name(inventory)));
        }
    }
    cells
}

fn row(
    resource: &ExportedResource,
    approved: &[(InventoryId, Vec<(ProductId, PriceIntent)>)],
) -> Vec<String> {
    let mut cells = vec![
        resource.id.0.to_hyphenated(),
        resource.title.0.clone(),
        amount(resource.price),
        denomination(resource.price).to_owned(),
        labels(&resource.labels),
        rfc3339(resource.created_at),
        rfc3339(resource.updated_at),
    ];
    for inventory in EXPORT_ORDER {
        // One listing at most, because `mapping_one_per_inventory` in
        // migration 0004 is unique over (org, product, inventory).
        let listing = resource
            .listings
            .iter()
            .find(|listing| listing.inventory == inventory);
        match listing {
            None => cells.extend([String::new(), String::new(), String::new()]),
            Some(listing) => cells.extend([
                standing(&listing.binding_state, &listing.lifecycle_state).to_owned(),
                listing_price(
                    approved_for(approved, inventory, resource.id).or(listing.listed_price),
                ),
                listing
                    .remote
                    .as_ref()
                    .and_then(listing_url)
                    .unwrap_or_default(),
            ]),
        }
    }
    cells
}

/// The price the seller approved for one resource on one marketplace, where
/// they approved one.
fn approved_for(
    approved: &[(InventoryId, Vec<(ProductId, PriceIntent)>)],
    inventory: InventoryId,
    product: ProductId,
) -> Option<PriceIntent> {
    approved
        .iter()
        .find(|(held, _)| *held == inventory)
        .and_then(|(_, rows)| rows.iter().find(|(subject, _)| *subject == product))
        .map(|(_, price)| *price)
}

/// The label set as one cell.
///
/// A semicolon inside a name is doubled, because the separator the
/// specification chose is a character a label may itself contain: without
/// doubling, one label named `Year 5; Algebra` and the two labels `Algebra`
/// and `Year 5` produce the same bytes, and a reader splitting the cell
/// reconstructs the wrong set. Doubling is the convention RFC 4180 already
/// uses for a quote inside a quoted field, so the escape is the one a CSV
/// reader's author will guess.
fn labels(names: &[String]) -> String {
    names
        .iter()
        .map(|name| name.replace(';', ";;"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Where one listing stands, in the vocabulary the console renders: this is
/// `standingOf` in `web/src/lib/tes-portfolio.ts`, over the same two stored
/// spellings `MappingHead` publishes to it.
///
/// A state neither this function nor the console recognises reads as `other`,
/// which is the fail-safe answer: a new binding state must not be exported as
/// live, and must not stop the document either.
fn standing(binding_state: &str, lifecycle_state: &str) -> &'static str {
    match (binding_state, lifecycle_state) {
        ("unbound", _) => "unsent",
        ("bound", "live") => "live",
        ("bound", "draft") => "draft",
        _ => "other",
    }
}

fn listing_price(listed: Option<PriceIntent>) -> String {
    match listed {
        None => String::new(),
        Some(PriceIntent::Free) => "Free".to_owned(),
        Some(PriceIntent::Paid(money)) => {
            format!("{} {}", major_units(money), money.currency().code())
        }
    }
}

fn amount(price: PriceIntent) -> String {
    match price {
        PriceIntent::Free => "Free".to_owned(),
        PriceIntent::Paid(money) => major_units(money),
    }
}

const fn denomination(price: PriceIntent) -> &'static str {
    match price {
        PriceIntent::Free => "",
        PriceIntent::Paid(money) => money.currency().code(),
    }
}

/// How many minor units make one major one. Total over the closed currency
/// set, so a denomination that does not count in hundredths has to say so here
/// rather than be rendered a hundred times its value.
const fn minor_units_per_major(currency: Currency) -> i64 {
    match currency {
        Currency::Gbp | Currency::Usd => 100,
    }
}

fn major_units(money: Money) -> String {
    let per_major = minor_units_per_major(money.currency());
    let whole = money.minor_units().div_euclid(per_major);
    let part = money.minor_units().rem_euclid(per_major);
    format!("{whole}.{part:02}")
}

/// One row, RFC 4180: fields comma-separated, the record ended by CRLF.
fn write_row(out: &mut String, cells: &[String]) {
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&field(cell));
    }
    out.push_str("\r\n");
}

/// One field: defused, then quoted if it needs to be.
///
/// A cell opening with a formula character is prefixed with an apostrophe,
/// which is what stops a spreadsheet from executing a seller's title as a
/// formula the moment the file is opened. The prefix is applied before the
/// quoting so a defused cell that also carries a comma still ends up one field.
fn field(raw: &str) -> String {
    let defused = if raw
        .chars()
        .next()
        .is_some_and(|first| matches!(first, '=' | '+' | '-' | '@' | '\t' | '\r'))
    {
        let mut prefixed = String::with_capacity(raw.len() + 1);
        prefixed.push('\'');
        prefixed.push_str(raw);
        prefixed
    } else {
        raw.to_owned()
    };
    if !defused.contains(['"', ',', '\r', '\n']) {
        return defused;
    }
    let mut quoted = String::with_capacity(defused.len() + 2);
    quoted.push('"');
    for character in defused.chars() {
        if character == '"' {
            quoted.push('"');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

/// The instant as RFC 3339, to the second.
///
/// Hand-written for the reason `paddle::instant_from_rfc3339` is: this crate
/// holds no date library, and the shape needed is one civil date and one
/// clock time in UTC.
fn rfc3339(at: Timestamp) -> String {
    let seconds = at.0.div_euclid(1_000);
    let day = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let (year, month, date) = civil_from_days(day);
    let hour = time.div_euclid(3_600);
    let minute = time.div_euclid(60).rem_euclid(60);
    let second = time.rem_euclid(60);
    format!("{year:04}-{month:02}-{date:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// The instant's civil date, which is what names the file.
fn civil_date(at: Timestamp) -> String {
    let (year, month, date) = civil_from_days(at.0.div_euclid(1_000).div_euclid(86_400));
    format!("{year:04}-{month:02}-{date:02}")
}

/// Hinnant's `civil_from_days`, the inverse of the `days_from_civil` the
/// Paddle instant parser reads with, exact over the range a stored timestamp
/// can hold.
const fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era = (day_of_era - day_of_era.div_euclid(1_460) + day_of_era.div_euclid(36_524)
        - day_of_era.div_euclid(146_096))
    .div_euclid(365);
    let year = year_of_era + era * 400;
    let day_of_year =
        day_of_era - (365 * year_of_era + year_of_era.div_euclid(4) - year_of_era.div_euclid(100));
    let month_phase = (5 * day_of_year + 2).div_euclid(153);
    let date = day_of_year - (153 * month_phase + 2).div_euclid(5) + 1;
    let month = if month_phase < 10 {
        month_phase + 3
    } else {
        month_phase - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, date)
}

#[cfg(test)]
mod tests {
    use super::{
        amount, civil_date, civil_from_days, field, header_row, labels, major_units, rfc3339,
        standing, write_row, EXPORT_ORDER,
    };
    use tam_types::{Currency, InventoryId, Money, PriceIntent, Timestamp};

    #[test]
    fn every_inventory_the_model_knows_gets_a_column_group() {
        for inventory in InventoryId::ALL {
            assert!(
                EXPORT_ORDER.contains(&inventory),
                "{inventory:?} carries listings but no column group exports them"
            );
        }
        assert_eq!(
            EXPORT_ORDER.len(),
            InventoryId::ALL.len(),
            "the export order names each inventory exactly once"
        );
    }

    #[test]
    fn the_header_names_a_column_for_every_cell_a_row_writes() {
        let header = header_row();
        assert_eq!(
            header.len(),
            16,
            "seven catalogue columns and three triples"
        );
        assert_eq!(header[7], "TES status");
        assert_eq!(header[9], "TES link");
        assert_eq!(header[11], "TPT price");
        assert_eq!(header[15], "Etsy link");
    }

    #[test]
    fn a_plain_field_is_written_bare_and_a_hostile_one_is_quoted() {
        assert_eq!(field("Fractions"), "Fractions");
        assert_eq!(field(""), "");
        assert_eq!(field("Fractions, decimals"), "\"Fractions, decimals\"");
        assert_eq!(field("the \"good\" one"), "\"the \"\"good\"\" one\"");
        assert_eq!(field("two\r\nlines"), "\"two\r\nlines\"");
    }

    #[test]
    fn a_field_a_spreadsheet_would_execute_is_defused() {
        assert_eq!(field("=1+1"), "'=1+1");
        assert_eq!(field("+44 worksheets"), "'+44 worksheets");
        assert_eq!(field("-5 off"), "'-5 off");
        assert_eq!(field("@everyone"), "'@everyone");
        assert_eq!(field("\tindented"), "'\tindented");
        assert_eq!(field("\rreturn"), "\"'\rreturn\"");
        assert_eq!(
            field("=SUM(A1,A2)"),
            "\"'=SUM(A1,A2)\"",
            "a defused cell carrying a comma is still one field"
        );
    }

    #[test]
    fn a_row_ends_in_a_carriage_return_and_a_line_feed() {
        let mut out = String::new();
        write_row(&mut out, &["one".to_owned(), "two".to_owned()]);
        assert_eq!(out, "one,two\r\n");
    }

    #[test]
    fn the_calendar_reads_back_the_dates_it_was_derived_from() {
        assert_eq!(rfc3339(Timestamp(0)), "1970-01-01T00:00:00Z");
        assert_eq!(
            rfc3339(Timestamp(1_788_607_353_000)),
            "2026-09-05T11:22:33Z"
        );
        assert_eq!(
            rfc3339(Timestamp(1_709_164_800_000)),
            "2024-02-29T00:00:00Z",
            "a leap day is a day like any other"
        );
        assert_eq!(civil_date(Timestamp(1_788_607_353_999)), "2026-09-05");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn a_price_is_written_in_the_units_a_seller_charges_in() {
        let paid = |minor| Money::new(minor, Currency::Gbp).ok();
        assert_eq!(paid(499).map(major_units).as_deref(), Some("4.99"));
        assert_eq!(paid(500).map(major_units).as_deref(), Some("5.00"));
        assert_eq!(paid(5).map(major_units).as_deref(), Some("0.05"));
    }

    /// Every stored binding state, against the lifecycle states that can
    /// accompany it, so the console's reading and this one cannot drift apart
    /// silently.
    #[test]
    fn a_listing_stands_where_the_console_says_it_does() {
        assert_eq!(standing("bound", "live"), "live");
        assert_eq!(standing("bound", "draft"), "draft");
        for lifecycle in ["absent", "submitted", "in_review", "rejected", "withdrawn"] {
            assert_eq!(
                standing("bound", lifecycle),
                "other",
                "bound and {lifecycle} is neither live nor unsent"
            );
        }
        assert_eq!(standing("unbound", "absent"), "unsent");
        assert_eq!(
            standing("unbound", "live"),
            "unsent",
            "an unbound mapping is unsent whatever lifecycle the row carries"
        );
        for binding in ["creating", "ambiguous_create", "severed"] {
            assert_eq!(standing(binding, "live"), "other");
        }
        assert_eq!(
            standing("a state nobody has written yet", "live"),
            "other",
            "an unknown state reads as other rather than as live"
        );
    }

    #[test]
    fn a_label_carrying_the_separator_stays_one_label() {
        assert_eq!(
            labels(&["Algebra".to_owned(), "Year 5".to_owned()]),
            "Algebra; Year 5"
        );
        assert_eq!(
            labels(&["Year 5; Algebra".to_owned()]),
            "Year 5;; Algebra",
            "one label carrying the separator is not two labels"
        );
        assert_eq!(labels(&[]), "");
    }

    /// The injection prefix fires on a leading `-`, and a price is the one
    /// column a seller would rather have as a number than as text. Nothing
    /// here needs an exemption, and this pins why: a price cannot be negative,
    /// because `Money::new` refuses a non-positive amount and every stored
    /// price is decoded through it. The invariant lives two crates away, so it
    /// is asserted here where the document depends on it.
    #[test]
    fn a_price_never_opens_with_a_character_a_spreadsheet_would_defuse() {
        assert!(
            Money::new(-499, Currency::Gbp).is_err(),
            "a negative amount is not a price"
        );
        assert!(
            Money::new(0, Currency::Gbp).is_err(),
            "zero is the Free variant, not a paid amount"
        );
        for minor in [1, 5, 99, 499, 100_000] {
            let Ok(money) = Money::new(minor, Currency::Usd) else {
                panic!("{minor} minor units is a price");
            };
            let cell = amount(PriceIntent::Paid(money));
            assert_eq!(
                field(&cell),
                cell,
                "a price cell is written bare, so it stays a number in a spreadsheet"
            );
        }
        assert_eq!(amount(PriceIntent::Free), "Free");
    }
}
