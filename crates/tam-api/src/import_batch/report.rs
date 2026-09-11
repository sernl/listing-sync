//! What each row of a parsed sheet says, and what is wrong with it.
//!
//! The report is the deliverable rather than a byproduct. A refused upload
//! creates nothing, and what the seller gets back is a table of sheet, row,
//! column and problem, where the row is their own spreadsheet row number so
//! they can go to it. Every message here is either the create form's own,
//! borrowed through `tam_authoring::refusal_of` so the wording cannot drift
//! from the form's, or a rule that exists only on this path and is stated once.
//!
//! Two things this module deliberately does not do. It contacts no database, so
//! a report is computable from the bytes and the registry alone and a parse
//! costs no statement. And it decides nothing about admissibility that the
//! registry already decides: requiredness, vocabularies and caps are read, not
//! restated.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tam_authoring::refusal_of;
use tam_domain::product::ProductName;

use crate::resources::{label_refusal, LABELS_PER_PRODUCT_MAX};
use tam_domain::registry::truncate;
use tam_storage::RowIntent;
use tam_types::{Currency, InventoryId, LengthUnit, Money, PriceIntent};

use super::parse::{Grid, GridRow, Malformed};
use super::sheet::{
    columns, currency_of, example_row, Cell, Column, Values, STATUS_DRAFT, STATUS_LIVE,
};

/// One refusal against one cell of one row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// The column's own header, as the seller reads it in row one.
    pub column: String,
    pub problem: String,
}

impl Problem {
    fn new(column: &str, problem: impl Into<String>) -> Self {
        Self {
            column: column.to_owned(),
            problem: problem.into(),
        }
    }
}

/// The parsed row, in the shape the commit reads back to build its create.
///
/// Held as a document rather than as columns for migration 0056's stated
/// reason: the field set is the create form's, the create form's is TPT's, and
/// TPT's moves. What keeps it honest is that this module builds it through the
/// same smart constructors the create route applies, so a document that reaches
/// the column is a document the create would accept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowDraft {
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub price: PriceIntent,
    #[serde(default)]
    pub labels: Vec<String>,
    /// The inventory this row is authored for, as `CreateProductBody` takes it:
    /// empty on the platform-only tab, one entry on a grid tab.
    #[serde(default)]
    pub inventories: Vec<InventoryId>,
    /// The tab inventory's own native fields, keyed by wire name. A field the
    /// seller left blank is absent rather than empty.
    #[serde(default)]
    pub natives: BTreeMap<String, Vec<String>>,
    /// The filename the seller's `File` column named, kept in the document as
    /// well as on the row so a resumed commit reads one thing.
    #[serde(default)]
    pub file_name: Option<String>,
}

/// One row after validation.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedRow {
    pub sheet: String,
    /// The seller's own spreadsheet row number.
    pub ordinal: u32,
    pub inventory: Option<InventoryId>,
    pub intent: RowIntent,
    pub draft: RowDraft,
    pub problems: Vec<Problem>,
    pub file_name: Option<String>,
}

/// The whole upload after validation.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub rows: Vec<ParsedRow>,
    /// Every label the sheet names, with how many rows carry it, ordered by
    /// count and then by name.
    ///
    /// The seller sees this before pressing import. A one-row label sitting
    /// beside a three-hundred-row near-identical one is the typo signature,
    /// rendered without anyone implementing fuzzy matching: a label the sheet
    /// would newly create is created, and creating it silently on four hundred
    /// rows is what this list exists to stop.
    pub labels: Vec<(String, u32)>,
}

/// Validates every grid of an upload.
///
/// A structural fault — a column the template does not write, a required column
/// the sheet does not carry — refuses the upload rather than every row of it:
/// a five-hundred-line report saying the same thing five hundred times is not a
/// report, and no batch should be written for a sheet whose shape is wrong.
pub fn report(grids: &[Grid]) -> Result<Report, Malformed> {
    let mut rows = Vec::new();
    let mut counted: BTreeMap<String, u32> = BTreeMap::new();
    for grid in grids {
        // A tab with neither a header nor a row is one the seller did not
        // fill, and an upload is allowed to hold several: every workbook this
        // template writes has five grid tabs and a seller uses the ones they
        // need. Asking such a tab for its columns would refuse the upload for
        // a tab nobody typed in.
        if grid.rows.is_empty() && grid.header.iter().all(String::is_empty) {
            continue;
        }
        let held = columns(grid.tab);
        let places = header_places(grid, &held)?;
        let example = example_row(grid.tab);
        for row in &grid.rows {
            if is_example(row, &places, &example) {
                continue;
            }
            let parsed = parse_row(grid, &held, &places, row);
            for label in &parsed.draft.labels {
                *counted.entry(label.clone()).or_insert(0) += 1;
            }
            rows.push(parsed);
        }
    }
    if rows.is_empty() {
        // After the shape check, not before it: a sheet missing a column holds
        // no readable rows either, and "every tab is empty" would send the
        // seller to fix the wrong thing.
        return Err(Malformed::NoRows);
    }
    let mut labels: Vec<(String, u32)> = counted.into_iter().collect();
    labels.sort_by(|(left_name, left), (right_name, right)| {
        right.cmp(left).then_with(|| left_name.cmp(right_name))
    });
    Ok(Report { rows, labels })
}

/// Which file column each of the tab's columns sits in.
///
/// Matched on the header text rather than on position, so a seller who moved a
/// column is read correctly rather than silently misread — which is the failure
/// a position-based mapping has and this does not.
fn header_places(grid: &Grid, held: &[Column]) -> Result<Vec<usize>, Malformed> {
    let mut places = Vec::with_capacity(held.len());
    for column in held {
        let found = grid
            .header
            .iter()
            .position(|cell| cell.trim().eq_ignore_ascii_case(&column.title));
        match found {
            Some(at) => places.push(at),
            None => {
                return Err(Malformed::ColumnMissing {
                    sheet: grid.tab.title.to_owned(),
                    column: column.title.clone(),
                })
            }
        }
    }
    for cell in &grid.header {
        let named = cell.trim();
        if named.is_empty() {
            continue;
        }
        if !held
            .iter()
            .any(|column| column.title.eq_ignore_ascii_case(named))
        {
            return Err(Malformed::ColumnUnknown {
                sheet: grid.tab.title.to_owned(),
                column: named.to_owned(),
            });
        }
    }
    Ok(places)
}

/// Whether this row is the template's own worked example, left as written.
///
/// Compared cell by cell against what the writer put there, so a seller who
/// typed over the example is read as data rather than losing a row to a
/// position-based skip. The example lives above the first data row in a
/// workbook this template wrote, so this only matters for a sheet a seller
/// rearranged or a `.csv` they exported from one.
fn is_example(row: &GridRow, places: &[usize], example: &[String]) -> bool {
    places.iter().enumerate().all(|(index, at)| {
        let held = row.cells.get(*at).map(String::as_str).unwrap_or_default();
        let expected = example.get(index).map(String::as_str).unwrap_or_default();
        held == expected
    })
}

fn cell_of<'a>(row: &'a GridRow, places: &[usize], index: usize) -> &'a str {
    places
        .get(index)
        .and_then(|at| row.cells.get(*at))
        .map(String::as_str)
        .unwrap_or_default()
}

#[expect(
    clippy::too_many_lines,
    reason = "one column per arm over a closed Cell, which is what makes a column added to the writer a compile error here; splitting it per arm would move the totality check out of one place"
)]
fn parse_row(grid: &Grid, held: &[Column], places: &[usize], row: &GridRow) -> ParsedRow {
    let mut problems: Vec<Problem> = Vec::new();
    let mut natives: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut title = String::new();
    let mut body = String::new();
    let mut labels: Vec<String> = Vec::new();
    let mut file_name: Option<String> = None;
    let mut price_text = String::new();
    let mut currency_text = String::new();
    let mut intent = RowIntent::Draft;

    for (index, column) in held.iter().enumerate() {
        let raw = cell_of(row, places, index).trim();
        match column.cell {
            Cell::Status => {
                intent = match raw.to_ascii_lowercase().as_str() {
                    "" | STATUS_DRAFT => RowIntent::Draft,
                    STATUS_LIVE if grid.tab.admits_live() => RowIntent::Live,
                    STATUS_LIVE => {
                        problems.push(Problem::new(
                            &column.title,
                            "this tab has no marketplace, so a row here cannot be live; leave the \
                             status blank or write \"draft\"",
                        ));
                        RowIntent::Draft
                    }
                    _ => {
                        problems.push(Problem::new(
                            &column.title,
                            format!("a status is \"{STATUS_DRAFT}\", \"{STATUS_LIVE}\" or blank"),
                        ));
                        RowIntent::Draft
                    }
                };
            }
            Cell::ResourceId => {
                if !raw.is_empty() {
                    problems.push(Problem::new(
                        &column.title,
                        "this import creates new resources only; leave the resource ID blank. \
                         Editing an existing resource from a spreadsheet is not supported yet",
                    ));
                }
            }
            Cell::Title => match ProductName::new(raw) {
                Ok(name) => {
                    name.as_str().clone_into(&mut title);
                    if let Values::Text { cap: Some(cap) } = column.values {
                        // A second cap in the platform's own counting unit,
                        // enforced beside the form's rather than instead of it:
                        // two caps in different units cannot be compared, so
                        // both are checked in their own.
                        if truncate(&title, cap) != title {
                            problems.push(Problem::new(
                                &column.title,
                                format!(
                                    "this marketplace holds a title to {} {}",
                                    cap.limit,
                                    unit_name(cap.unit)
                                ),
                            ));
                        }
                    }
                }
                Err(error) => {
                    problems.push(Problem::new(&column.title, refusal_of(&error).message));
                }
            },
            Cell::Description => {
                raw.clone_into(&mut body);
                if let Values::Text { cap: Some(cap) } = column.values {
                    if truncate(&body, cap) != body {
                        problems.push(Problem::new(
                            &column.title,
                            format!(
                                "this marketplace holds a description to {} {}",
                                cap.limit,
                                unit_name(cap.unit)
                            ),
                        ));
                    }
                }
            }
            Cell::Price => raw.clone_into(&mut price_text),
            Cell::Currency => raw.clone_into(&mut currency_text),
            Cell::Labels => match parse_labels(raw) {
                Ok(names) => labels = names,
                Err(refusal) => problems.push(Problem::new(&column.title, refusal)),
            },
            Cell::File => {
                if !raw.is_empty() {
                    file_name = Some(raw.to_owned());
                }
            }
            Cell::Native(name) => {
                let values = if column.multiple {
                    split_set(raw)
                } else if raw.is_empty() {
                    Vec::new()
                } else {
                    vec![raw.to_owned()]
                };
                if values.is_empty() {
                    if column.required {
                        problems.push(Problem::new(
                            &column.title,
                            "this marketplace refuses a resource without this field",
                        ));
                    }
                    continue;
                }
                let mut kept = Vec::with_capacity(values.len());
                for value in values {
                    match admissible(column, &value) {
                        Ok(value) => kept.push(value),
                        Err(refusal) => problems.push(Problem::new(&column.title, refusal)),
                    }
                }
                if !kept.is_empty() {
                    natives.insert(name.to_owned(), kept);
                }
            }
        }
    }

    // Price after the loop, because it needs the currency column that may sit
    // to its right and the intent that decides nothing about it but reads
    // better beside it.
    let currency = match grid.tab.inventory.map(inventory_currency) {
        Some(Some(currency)) => Ok(currency),
        Some(None) | None => match currency_text.to_ascii_uppercase().as_str() {
            "" => Err(()),
            code => currency_of(code).ok_or(()),
        },
    };
    let price = match parse_price(&price_text, currency) {
        Ok(price) => price,
        Err(refusal) => {
            let column = held
                .iter()
                .find(|column| column.cell == Cell::Price)
                .map_or("Price", |column| column.title.as_str());
            problems.push(Problem::new(column, refusal));
            PriceIntent::Free
        }
    };

    // Decision D32: the file follows the destination rather than the lifecycle.
    // A row naming a marketplace publishes a file there whether it asks for a
    // draft on that marketplace or a live listing, and `create_product` refuses
    // a create that names an inventory and carries no payload by its own name,
    // before the general payload refusal. A Teachouse row names no marketplace
    // and needs no file at all.
    //
    // Refusing on the intent instead would let a marketplace draft row through
    // a clean report and fail it at commit, which is the failure the
    // reject-the-whole-upload model exists to prevent.
    if grid.tab.inventory.is_some() && file_name.is_none() {
        let column = held
            .iter()
            .find(|column| column.cell == Cell::File)
            .map_or("File", |column| column.title.as_str());
        problems.push(Problem::new(
            column,
            "a resource on a marketplace needs a file buyers can download; name the file here \
             and attach it after the upload",
        ));
    }

    ParsedRow {
        sheet: grid.tab.title.to_owned(),
        ordinal: row.number,
        inventory: grid.tab.inventory,
        intent,
        draft: RowDraft {
            title,
            body,
            price,
            labels: labels.clone(),
            inventories: grid.tab.inventory.into_iter().collect(),
            natives,
            file_name: file_name.clone(),
        },
        problems,
        file_name,
    }
}

/// The denomination one inventory fixes, or none where it fixes none.
///
/// Read off the inventory rather than restated, so the sheet cannot disagree
/// with what the create writes. `SellerScoped` and `Unmeasured` answer none,
/// and no tab reaches them: `TABS` holds only inventories this build publishes
/// to, and all four fix a currency.
fn inventory_currency(inventory: InventoryId) -> Option<Currency> {
    match inventory.currency_rule() {
        tam_types::CurrencyRule::Fixed(currency) => Some(currency),
        tam_types::CurrencyRule::SellerScoped | tam_types::CurrencyRule::Unmeasured => None,
    }
}

/// How many minor units make one major one.
///
/// An exhaustive match over the closed currency set, which is the forcing
/// function: a denomination that does not count in hundredths has to say so
/// here rather than have a seller's price read a hundred times its value.
/// `export.rs` states the same fact for the same reason at the other end of the
/// round trip.
const fn minor_units_per_major(currency: Currency) -> i64 {
    match currency {
        Currency::Gbp | Currency::Usd => 100,
    }
}

/// The price a cell states, or the refusal a seller can act on.
///
/// `Err(())` for the currency means the row states no denomination and none
/// follows from its tab, which matters only for a paid price: a free row needs
/// no currency and states none.
fn parse_price(raw: &str, currency: Result<Currency, ()>) -> Result<PriceIntent, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err("every row states a price, or the word \"free\"".to_owned());
    }
    if text.eq_ignore_ascii_case("free") {
        return Ok(PriceIntent::Free);
    }
    let Ok(currency) = currency else {
        return Err(
            "a paid row states the currency its price is in, in the Currency column".to_owned(),
        );
    };
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.chars().all(|character| character.is_ascii_digit())
        || !fraction.chars().all(|character| character.is_ascii_digit())
    {
        return Err(
            "a price is a number like 3.50, or the word \"free\"; write no currency symbol"
                .to_owned(),
        );
    }
    if fraction.len() > 2 {
        return Err("a price is stated to at most two decimal places".to_owned());
    }
    let per_major = minor_units_per_major(currency);
    let Ok(major) = whole.parse::<i64>() else {
        return Err("that price is larger than any price this system holds".to_owned());
    };
    let padded = format!("{fraction:0<2}");
    let minor: i64 = padded.parse().unwrap_or(0);
    let Some(amount) = major
        .checked_mul(per_major)
        .and_then(|at| at.checked_add(minor))
    else {
        return Err("that price is larger than any price this system holds".to_owned());
    };
    Money::new(amount, currency).map(PriceIntent::Paid).map_err(|_| {
        // `Money::new` refuses zero and below, and its own error carries no
        // seller-facing rendering; the console states the same rule in the
        // same words on its edit form.
        "a paid price is a positive amount, written in the currency's own units, or the word \"free\""
            .to_owned()
    })
}

/// Whether a value is one this column admits, and the spelling it is stored
/// under.
///
/// A captured closed set is matched case-insensitively and stored in the
/// registry's own spelling, so a seller who typed `cc-by` writes `CC-BY`. The
/// other three answers accept what they are given, each for its own reason: a
/// set documented closed whose members we do not hold has nothing to check
/// against, and inventing a check would refuse a value the platform accepts.
fn admissible(column: &Column, value: &str) -> Result<String, String> {
    match column.values {
        Values::Closed(members) => members
            .iter()
            .find(|member| member.eq_ignore_ascii_case(value))
            .map(|member| (*member).to_owned())
            .ok_or_else(|| {
                format!(
                    "\"{value}\" is not one of the values this column offers: {}",
                    members.join(", ")
                )
            }),
        Values::Numeric => {
            if value.chars().all(|character| character.is_ascii_digit()) {
                Ok(value.to_owned())
            } else {
                Err(format!("\"{value}\" is not a number"))
            }
        }
        Values::ClosedUncaptured | Values::Text { .. } => Ok(value.to_owned()),
    }
}

/// A semicolon-separated cell as its members, in the export's own convention: a
/// literal semicolon inside a value is doubled, so it is halved here.
fn split_set(raw: &str) -> Vec<String> {
    let mut values = split_tokens(raw);
    values.retain(|value| !value.is_empty());
    values
}

/// The same split without discarding empty tokens.
///
/// A native field's empty token is noise and is dropped by [`split_set`]; a
/// label's is a refusal, because the label route refuses a name with no word in
/// it. The two callers want different things from the same separator, so the
/// split is one function and the discarding is the caller's.
fn split_tokens(raw: &str) -> Vec<String> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let mut values: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut characters = raw.chars().peekable();
    while let Some(character) = characters.next() {
        if character != ';' {
            current.push(character);
            continue;
        }
        if characters.peek() == Some(&';') {
            let _consumed = characters.next();
            current.push(';');
            continue;
        }
        values.push(core::mem::take(&mut current).trim().to_owned());
    }
    values.push(current.trim().to_owned());
    values
}

/// The labels a cell names, or the refusal a seller can act on.
///
/// The rules are the label route's own, reached rather than restated:
/// [`label_refusal`] holds them and [`LABELS_PER_PRODUCT_MAX`] bounds the set,
/// both borrowed from `crate::resources`. One home rather than two copies that
/// agree today — a bound moving there moves here, which is the property a
/// second copy cannot have however carefully it is written.
///
/// A label the organisation does not hold is created rather than refused, which
/// is what `LabelRepo::set_for_product` already does; the list of which ones
/// would be new is the report's own.
fn parse_labels(raw: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    // `split_tokens` rather than `split_set`, so an empty token survives the
    // split and reaches the rules. A stray or doubled separator is a label the
    // seller did not finish typing, and the label route refuses it by name; a
    // bulk path that silently dropped it would be the one place this module
    // is more permissive than the form it claims to mirror.
    for value in split_tokens(raw) {
        let name = label_refusal(&value)?;
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if names.len() > LABELS_PER_PRODUCT_MAX {
        return Err(format!(
            "a resource carries at most {LABELS_PER_PRODUCT_MAX} labels, and this row names {}",
            names.len()
        ));
    }
    Ok(names)
}

/// A cap's counting unit in the words a seller reads, since a cap stated in
/// UTF-16 code units means nothing to the person filling the cell.
const fn unit_name(unit: LengthUnit) -> &'static str {
    match unit {
        LengthUnit::Bytes => "bytes",
        LengthUnit::Utf16CodeUnits | LengthUnit::Codepoints => "characters",
        LengthUnit::GraphemeClusters => "letters",
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_labels, parse_price, report, split_set, LABELS_PER_PRODUCT_MAX};
    use crate::import_batch::parse::{grids, Malformed};
    use crate::import_batch::sheet::{columns, example_row, tab_of, Cell};
    use tam_storage::RowIntent;
    use tam_types::{Currency, Money, PriceIntent};

    /// One tab as a comma-separated document: the header, the marker row, the
    /// example, then whatever rows the test names.
    fn sheet(tab_title: &str, rows: &[&[(Cell, &str)]]) -> String {
        let Some(tab) = tab_of(tab_title) else {
            panic!("{tab_title} is a tab");
        };
        let held = columns(tab);
        let field = |value: &str| -> String {
            if value.contains([',', '"']) {
                format!("\"{}\"", value.replace('"', "\"\""))
            } else {
                value.to_owned()
            }
        };
        let mut records = vec![
            held.iter()
                .map(|column| field(&column.title))
                .collect::<Vec<_>>()
                .join(","),
            held.iter()
                .map(|column| column.marker.as_str().to_owned())
                .collect::<Vec<_>>()
                .join(","),
            example_row(tab)
                .iter()
                .map(|value| field(value))
                .collect::<Vec<_>>()
                .join(","),
        ];
        for row in rows {
            records.push(
                held.iter()
                    .map(|column| {
                        field(
                            row.iter()
                                .find(|(cell, _)| *cell == column.cell)
                                .map_or("", |(_, value)| *value),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        records.join("\r\n")
    }

    fn read(tab_title: &str, rows: &[&[(Cell, &str)]]) -> super::Report {
        let document = sheet(tab_title, rows);
        let name = format!("{tab_title}.csv");
        let Ok(read) = grids(&name, document.as_bytes()) else {
            panic!("the fixture sheet parses");
        };
        let Ok(reported) = report(&read) else {
            panic!("the fixture sheet validates structurally");
        };
        reported
    }

    #[test]
    fn a_complete_row_passes_and_carries_what_the_create_will_send() {
        let reported = read(
            "TES",
            &[&[
                (Cell::Title, "Fractions of amounts"),
                (Cell::Price, "3.50"),
                (Cell::Labels, "Autumn Term; Year 5"),
                (Cell::File, "fractions.pdf"),
                (Cell::Native("licence"), "CC-BY"),
            ]],
        );
        let Some(row) = reported.rows.first() else {
            panic!("the one row is present");
        };
        assert_eq!(row.problems, vec![], "a complete row draws no refusal");
        assert_eq!(row.ordinal, 4, "the row cites the number the seller reads");
        assert_eq!(
            row.intent,
            RowIntent::Draft,
            "a blank status reads as draft"
        );
        assert_eq!(
            row.draft.price,
            PriceIntent::Paid(
                Money::new(350, Currency::Gbp).unwrap_or_else(|_| panic!("350 pence is a price"))
            ),
            "the price is minor units in the currency the tab fixes"
        );
        assert_eq!(
            row.draft.labels,
            vec!["Autumn Term".to_owned(), "Year 5".to_owned()]
        );
        assert_eq!(
            row.draft.natives.get("licence"),
            Some(&vec!["CC-BY".to_owned()])
        );
    }

    #[test]
    fn a_live_row_without_a_file_is_refused_naming_the_file_column() {
        let reported = read(
            "TES",
            &[&[
                (Cell::Status, "live"),
                (Cell::Title, "A worksheet"),
                (Cell::Price, "free"),
                (Cell::Native("licence"), "CC-BY"),
            ]],
        );
        let Some(row) = reported.rows.first() else {
            panic!("the one row is present");
        };
        assert_eq!(
            row.problems
                .iter()
                .map(|p| p.column.as_str())
                .collect::<Vec<_>>(),
            vec!["File"],
            "the one refusal names the column to fix: {:?}",
            row.problems
        );
    }

    /// Decision D32, the half a lifecycle-shaped rule would miss: a row that
    /// asks only for a draft still names a marketplace, and a resource on a
    /// marketplace needs a file buyers can download.
    ///
    /// The severity is that `create_product` refuses exactly this at commit,
    /// by its own name, before the general payload refusal. Without this the
    /// row would pass a clean report and fail the import, which is the failure
    /// the reject-the-whole-upload model exists to prevent.
    #[test]
    fn a_marketplace_draft_row_without_a_file_is_refused_like_a_live_one() {
        let reported = read(
            "TES",
            &[&[
                (Cell::Status, "draft"),
                (Cell::Title, "A worksheet"),
                (Cell::Price, "free"),
                (Cell::Native("licence"), "CC-BY"),
            ]],
        );
        let Some(row) = reported.rows.first() else {
            panic!("the one row is present");
        };
        assert_eq!(row.intent, RowIntent::Draft, "the row asked for a draft");
        assert_eq!(
            row.problems
                .iter()
                .map(|problem| problem.column.as_str())
                .collect::<Vec<_>>(),
            vec!["File"],
            "the file follows the destination, not the lifecycle: {:?}",
            row.problems
        );
    }

    /// The other half of D32: a row naming no marketplace needs no file, which
    /// is what makes the Teachouse tab a planning surface rather than a second
    /// upload queue.
    #[test]
    fn a_platform_only_row_without_a_file_passes() {
        let reported = read(
            "Teachouse",
            &[&[(Cell::Title, "A planning grid"), (Cell::Price, "free")]],
        );
        assert_eq!(
            reported.rows.first().map(|row| row.problems.clone()),
            Some(vec![]),
            "a row that names no marketplace publishes nowhere, so it needs no file"
        );
    }

    #[test]
    fn a_required_marketplace_field_left_blank_is_refused_off_the_registry() {
        let reported = read(
            "TES",
            &[&[(Cell::Title, "A worksheet"), (Cell::Price, "free")]],
        );
        let Some(row) = reported.rows.first() else {
            panic!("the one row is present");
        };
        assert!(
            row.problems
                .iter()
                .any(|problem| problem.column == "Licence"),
            "the Tes licence is the registry's one hard-required native: {:?}",
            row.problems
        );
    }

    #[test]
    fn a_value_outside_a_captured_set_is_refused_and_the_set_is_listed() {
        let reported = read(
            "TES",
            &[&[
                (Cell::Title, "A worksheet"),
                (Cell::Price, "free"),
                (Cell::File, "worksheet.pdf"),
                (Cell::Native("licence"), "CC-WHATEVER"),
            ]],
        );
        let Some(row) = reported.rows.first() else {
            panic!("the one row is present");
        };
        let Some(problem) = row.problems.first() else {
            panic!("the row is refused");
        };
        assert!(
            problem.problem.contains("CC-BY"),
            "the refusal states what would have worked: {}",
            problem.problem
        );
    }

    #[test]
    fn a_captured_value_is_stored_in_the_registrys_own_spelling() {
        let reported = read(
            "TES",
            &[&[
                (Cell::Title, "A worksheet"),
                (Cell::Price, "free"),
                (Cell::File, "worksheet.pdf"),
                (Cell::Native("licence"), "cc-by"),
            ]],
        );
        let Some(row) = reported.rows.first() else {
            panic!("the one row is present");
        };
        assert_eq!(
            row.draft.natives.get("licence"),
            Some(&vec!["CC-BY".to_owned()]),
            "a seller's lowercase spelling is stored as the platform spells it"
        );
    }

    #[test]
    fn the_platform_only_tab_refuses_a_live_row_and_needs_a_currency_when_paid() {
        let reported = read(
            "Teachouse",
            &[
                &[
                    (Cell::Status, "live"),
                    (Cell::Title, "A worksheet"),
                    (Cell::Price, "free"),
                ],
                &[(Cell::Title, "Another"), (Cell::Price, "2.00")],
                &[
                    (Cell::Title, "A third"),
                    (Cell::Price, "2.00"),
                    (Cell::Currency, "GBP"),
                ],
            ],
        );
        let columns_refused: Vec<Vec<&str>> = reported
            .rows
            .iter()
            .map(|row| row.problems.iter().map(|p| p.column.as_str()).collect())
            .collect();
        assert_eq!(
            columns_refused,
            vec![vec!["Status"], vec!["Price"], Vec::<&str>::new()],
            "live is refused on a tab with no marketplace, and a paid row states its currency"
        );
    }

    #[test]
    fn a_resource_id_is_refused_rather_than_quietly_ignored() {
        let reported = read(
            "Teachouse",
            &[&[
                (Cell::ResourceId, "11111111-1111-4111-8111-111111111111"),
                (Cell::Title, "A worksheet"),
                (Cell::Price, "free"),
            ]],
        );
        let Some(row) = reported.rows.first() else {
            panic!("the one row is present");
        };
        assert_eq!(
            row.problems
                .iter()
                .map(|p| p.column.as_str())
                .collect::<Vec<_>>(),
            vec!["Resource ID"],
            "update by spreadsheet is refused by name rather than silently dropped"
        );
    }

    #[test]
    fn the_example_row_left_as_written_is_not_imported() {
        let document = sheet("TES", &[]);
        let Ok(read) = grids("TES.csv", document.as_bytes()) else {
            panic!("the fixture sheet parses");
        };
        assert_eq!(
            report(&read).err(),
            Some(Malformed::NoRows),
            "an untouched example row is not a resource, so the sheet states nothing"
        );
    }

    #[test]
    fn an_example_row_typed_over_is_the_sellers_own_row() {
        // The sheet helper writes the example at row three and this row at
        // four; overwriting the example's own position is what a seller does
        // when they start typing in it, so a row equal to the example except
        // in one cell has to be read as data.
        let mut document = sheet("TES", &[]);
        let Some(tab) = tab_of("TES") else {
            panic!("TES is a tab");
        };
        let held = columns(tab);
        let Some(title_at) = held.iter().position(|column| column.cell == Cell::Title) else {
            panic!("every tab has a title column");
        };
        let mut example = example_row(tab);
        if let Some(slot) = example.get_mut(title_at) {
            *slot = "My own title".to_owned();
        }
        document.push_str("\r\n");
        document.push_str(&example.join(","));
        let Ok(read) = grids("TES.csv", document.as_bytes()) else {
            panic!("the fixture sheet parses");
        };
        let Ok(reported) = report(&read) else {
            panic!("the fixture sheet validates structurally");
        };
        assert_eq!(
            reported.rows.len(),
            1,
            "a row differing from the example in one cell is the seller's own"
        );
    }

    #[test]
    fn a_column_the_template_does_not_write_refuses_the_upload() {
        let document = format!("{}\r\n", sheet("TES", &[]));
        let with_extra = document.replace("Status,", "Status,My notes,");
        assert_eq!(
            grids("TES.csv", with_extra.as_bytes())
                .and_then(|read| report(&read))
                .err(),
            Some(Malformed::ColumnUnknown {
                sheet: "TES".to_owned(),
                column: "My notes".to_owned()
            }),
            "an unrecognised column is named rather than ignored, because a seller who filled \
             it in expected it to matter"
        );
    }

    #[test]
    fn a_missing_column_refuses_the_upload_rather_than_every_row_of_it() {
        let document = sheet("TES", &[]);
        let without_title = document.replacen("Title,", "", 1);
        assert!(
            matches!(
                grids("TES.csv", without_title.as_bytes()).and_then(|read| report(&read)),
                Err(Malformed::ColumnMissing { .. })
            ),
            "a sheet whose shape is wrong writes no batch at all"
        );
    }

    #[test]
    fn the_labels_the_sheet_would_create_are_counted_for_the_seller_to_see() {
        let reported = read(
            "Teachouse",
            &[
                &[
                    (Cell::Title, "One"),
                    (Cell::Price, "free"),
                    (Cell::Labels, "Autumn Term"),
                ],
                &[
                    (Cell::Title, "Two"),
                    (Cell::Price, "free"),
                    (Cell::Labels, "Autumn Term; Spring"),
                ],
                &[
                    (Cell::Title, "Three"),
                    (Cell::Price, "free"),
                    (Cell::Labels, "Autum Term"),
                ],
            ],
        );
        assert_eq!(
            reported.labels,
            vec![
                ("Autumn Term".to_owned(), 2),
                ("Autum Term".to_owned(), 1),
                ("Spring".to_owned(), 1),
            ],
            "the typo signature is a one-row label beside a near-identical many-row one, \
             rendered without fuzzy matching"
        );
    }

    #[test]
    fn a_semicolon_inside_a_label_survives_the_export_convention() {
        assert_eq!(
            split_set("Year 5;; Algebra; Autumn"),
            vec!["Year 5; Algebra".to_owned(), "Autumn".to_owned()],
            "a doubled semicolon is one literal semicolon, which is what export.rs writes"
        );
    }

    /// S2: an empty token is a refusal here because it is a refusal on the
    /// label route, rather than being quietly dropped as it was.
    #[test]
    fn an_empty_label_token_is_refused_rather_than_dropped() {
        for cell in ["Autumn Term; ; Spring", "; Autumn Term", "Autumn Term;"] {
            assert_eq!(
                parse_labels(cell).err(),
                Some("a label needs a word in it".to_owned()),
                "\"{cell}\" names a label with no word in it, and the label route refuses that"
            );
        }
        // The doubling convention is not an empty token and must survive: `;;`
        // is one literal semicolon inside a name, which is what `export.rs`
        // writes and what this has to read back.
        assert_eq!(
            parse_labels("Autumn Term;;; Spring"),
            Ok(vec!["Autumn Term;".to_owned(), "Spring".to_owned()]),
            "a doubled separator is a character in a name, not a missing label"
        );
    }

    #[test]
    fn a_label_is_refused_by_the_same_rules_the_label_route_applies() {
        assert_eq!(
            parse_labels("a/b").err(),
            Some(
                "a label cannot contain a slash, because a label is addressed by its own name"
                    .to_owned()
            ),
            "the slash rule is the address path's, and the wording is the label route's"
        );
        let long = "x".repeat(61);
        assert!(
            parse_labels(&long).is_err(),
            "sixty characters is the bound migration 0046 states on the column"
        );
        let many = (0..=LABELS_PER_PRODUCT_MAX)
            .map(|index| format!("label {index}"))
            .collect::<Vec<_>>()
            .join("; ");
        assert!(
            parse_labels(&many).is_err(),
            "twenty labels per resource is the bound the label route applies"
        );
    }

    #[test]
    fn a_price_is_read_in_minor_units_and_refused_when_it_is_not_a_number() {
        for (text, expected) in [
            ("free", Ok(PriceIntent::Free)),
            ("FREE", Ok(PriceIntent::Free)),
            ("3", Money::new(300, Currency::Gbp).map(PriceIntent::Paid)),
            ("3.5", Money::new(350, Currency::Gbp).map(PriceIntent::Paid)),
            (
                "3.50",
                Money::new(350, Currency::Gbp).map(PriceIntent::Paid),
            ),
            ("0.99", Money::new(99, Currency::Gbp).map(PriceIntent::Paid)),
        ] {
            let Ok(expected) = expected else {
                panic!("the fixture price is a price");
            };
            assert_eq!(
                parse_price(text, Ok(Currency::Gbp)),
                Ok(expected),
                "\"{text}\" reads as {expected:?}"
            );
        }
        for text in ["", "£3.50", "3.505", "three", "-1", "0"] {
            assert!(
                parse_price(text, Ok(Currency::Gbp)).is_err(),
                "\"{text}\" is refused rather than guessed at"
            );
        }
    }
}
