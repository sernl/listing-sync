//! The template workbook, generated from the registry on request.
//!
//! Generated rather than checked in, so a field whose requiredness or
//! vocabulary changes in Rust changes the workbook with no one editing a
//! spreadsheet by hand. The forcing function is
//! [`super::sheet`], which the writer here and the parser beside it both read:
//! a native field added to the registry becomes a column in this workbook and a
//! cell the parser recognises in the same edit, or neither.
//!
//! Four regions per grid tab, which is the layout the parser expects and the
//! read-me states. Row 1 is the column names. Row 2 is the required marker. Row
//! 3 is one worked example, skipped on import while it is unchanged. Row 4
//! onward is the seller's data, and its Excel row number is the number a
//! refusal cites.

use rust_xlsxwriter::{
    DataValidation, DocProperties, ExcelDateTime, Format, Formula, Workbook, Worksheet, XlsxError,
};
use tam_types::{Currency, CurrencyRule, InventoryId};

use super::sheet::{
    columns, example_row, vocabularies, Cell, Column, Marker, Tab, Values, LISTS, READ_ME,
    STATUS_DRAFT, STATUS_LIVE, TABS,
};

/// The workbook row a grid tab's data starts on, counting from one as a seller
/// reads it: the header, the marker and the example occupy the first three.
pub const FIRST_DATA_ROW: u32 = 4;

/// The zero-based row index the writer puts the header on.
const HEADER_ROW: u32 = 0;
const MARKER_ROW: u32 = 1;
const EXAMPLE_ROW: u32 = 2;
const DATA_ROW: u32 = 3;

/// How far down each grid tab's dropdowns and formats reach.
///
/// The row ceiling rather than Excel's own million: a validation applied to
/// every row of a sheet is carried in the file whether or not a cell uses it,
/// and the bound the upload enforces is the honest place to stop. A seller who
/// pastes past it meets the row refusal rather than a cell with no dropdown.
fn last_data_row() -> u32 {
    let rows = u32::try_from(tam_limits::import::ROWS_PER_UPLOAD_MAX).unwrap_or(u32::MAX);
    DATA_ROW.saturating_add(rows)
}

/// The whole workbook as xlsx bytes.
pub fn workbook() -> Result<Vec<u8>, XlsxError> {
    let mut workbook = Workbook::new();
    workbook.set_properties(&fixed_properties()?);
    let lists = vocabularies();

    let mut read_me = Worksheet::new();
    read_me.set_name(READ_ME)?;
    write_read_me(&mut read_me)?;
    workbook.push_worksheet(read_me);

    for tab in TABS {
        let mut sheet = Worksheet::new();
        sheet.set_name(tab.title)?;
        write_grid(&mut sheet, tab, &lists)?;
        workbook.push_worksheet(sheet);
    }

    let mut list_sheet = Worksheet::new();
    list_sheet.set_name(LISTS)?;
    write_lists(&mut list_sheet, &lists)?;
    // Hidden rather than absent: Excel data validation over a long set wants a
    // range rather than an inline literal, so the members have to be on a sheet
    // — and a sheet of vocabulary tokens beside the seller's tabs is a tab they
    // would have to be told to ignore.
    list_sheet.set_hidden(true);
    workbook.push_worksheet(list_sheet);

    workbook.save_to_buffer()
}

/// Document properties with a fixed instant, so two downloads of the template
/// are the same bytes.
///
/// `DocProperties::new()` stamps `creation_time` with the real wall clock, and
/// both `dcterms:created` and `dcterms:modified` carry it into the archive, so
/// the default makes every generation a different file. Nothing in this feature
/// needs a timestamp — the workbook is a blank form derived from the registry,
/// not a record of when someone asked for one — and a form that differs from
/// itself is one no golden test can hold and no seller can diff.
///
/// The instant is the date this template was first generated from the registry.
/// It is arbitrary and it is fixed; those are the two properties it needs.
fn fixed_properties() -> Result<DocProperties, XlsxError> {
    let stamped = ExcelDateTime::from_ymd(2026, 9, 5)?;
    Ok(DocProperties::new().set_creation_datetime(&stamped))
}

fn write_grid(
    sheet: &mut Worksheet,
    tab: Tab,
    lists: &[(String, &'static [&'static str])],
) -> Result<(), XlsxError> {
    let header = Format::new().set_bold();
    let marker = Format::new().set_italic();
    let example = Format::new().set_italic();
    let held = columns(tab);
    let example_cells = example_row(tab);

    for (index, column) in held.iter().enumerate() {
        let at = u16::try_from(index).unwrap_or(u16::MAX);
        sheet.write_string_with_format(HEADER_ROW, at, &column.title, &header)?;
        sheet.write_string_with_format(MARKER_ROW, at, column.marker.as_str(), &marker)?;
        if let Some(cell) = example_cells.get(index) {
            sheet.write_string_with_format(EXAMPLE_ROW, at, cell, &example)?;
        }
        sheet.set_column_width(at, width_of(column))?;
        if let Values::Closed(members) = column.values {
            // The dropdown is advisory in the file and authoritative in the
            // parser: Excel warns, and the upload refuses. `ignore_blank` is on
            // because a blank cell is the ordinary state of an optional column,
            // and requiredness is stated by the marker row and enforced on
            // import rather than by a validation a seller can paste past.
            let validation = DataValidation::new()
                .allow_list_formula(Formula::new(range_of(lists, members)))
                .ignore_blank(true);
            sheet.add_data_validation(DATA_ROW, at, last_data_row(), at, &validation)?;
        }
    }
    // The header, the marker and the example stay on screen while the seller
    // scrolls, which is what makes a forty-column tab fillable at all.
    sheet.set_freeze_panes(DATA_ROW, 0)?;
    Ok(())
}

/// A column's width in characters, by what it holds rather than by measuring
/// the one example row: a title column sized to the example is a title column
/// that shrinks when the example is edited.
fn width_of(column: &Column) -> f64 {
    match column.cell {
        Cell::Description => 60.0,
        Cell::Title | Cell::Labels | Cell::File => 32.0,
        Cell::ResourceId => 38.0,
        Cell::Status | Cell::Price | Cell::Currency => 12.0,
        Cell::Native(_) => 22.0,
    }
}

/// The `Lists!$A$2:$A$n` range holding one captured set.
///
/// Falls back to the first column where the set is not listed, which
/// `every_captured_vocabulary_a_column_offers_is_written_to_the_lists_tab`
/// proves cannot happen: the fallback exists because a range is a string and
/// there is no total function from a set to one, not because the case is
/// reachable.
fn range_of(
    lists: &[(String, &'static [&'static str])],
    members: &'static [&'static str],
) -> String {
    let (index, held) = lists
        .iter()
        .enumerate()
        .find(|(_, (_, held))| *held == members)
        .map_or((0, members), |(index, (_, held))| (index, *held));
    let letter = column_letter(index);
    let last = held.len().saturating_add(1).max(2);
    format!("{LISTS}!${letter}$2:${letter}${last}")
}

/// Excel's own column naming: a bijective base-26 over the letters.
fn column_letter(index: usize) -> String {
    let mut letters = Vec::new();
    let mut remaining = index;
    loop {
        let digit = remaining % 26;
        let Some(letter) = char::from_u32(u32::try_from(digit).unwrap_or(0) + u32::from(b'A'))
        else {
            break;
        };
        letters.push(letter);
        if remaining < 26 {
            break;
        }
        // Bijective base twenty-six: the carry is one less than the quotient,
        // because there is no zero digit. `div_euclid` rather than `/` because
        // the lint table denies bare integer division, and both operands are
        // non-negative so the two agree.
        remaining = remaining.div_euclid(26).saturating_sub(1);
    }
    letters.reverse();
    letters.into_iter().collect()
}

fn write_lists(
    sheet: &mut Worksheet,
    lists: &[(String, &'static [&'static str])],
) -> Result<(), XlsxError> {
    let header = Format::new().set_bold();
    for (index, (title, members)) in lists.iter().enumerate() {
        let at = u16::try_from(index).unwrap_or(u16::MAX);
        sheet.write_string_with_format(0, at, title, &header)?;
        for (offset, member) in members.iter().enumerate() {
            let row = u32::try_from(offset).unwrap_or(u32::MAX).saturating_add(1);
            sheet.write_string(row, at, *member)?;
        }
    }
    Ok(())
}

/// The prose tab.
///
/// Every line here is a fact the workbook cannot carry in a cell: what a status
/// value does, what the file column points at, which currency each tab prices
/// in, where the seller's files go, and which columns hold a set we know is
/// closed without holding its members.
fn write_read_me(sheet: &mut Worksheet) -> Result<(), XlsxError> {
    let heading = Format::new().set_bold();
    let mut row = 0_u32;
    let mut write = |sheet: &mut Worksheet, text: &str, bold: bool| -> Result<(), XlsxError> {
        if bold {
            sheet.write_string_with_format(row, 0, text, &heading)?;
        } else {
            sheet.write_string(row, 0, text)?;
        }
        row = row.saturating_add(1);
        Ok(())
    };
    sheet.set_column_width(0, 110.0)?;

    write(sheet, "Teachouse import template", true)?;
    write(sheet, "", false)?;
    write(
        sheet,
        "One tab per place a resource can be listed, plus a Teachouse tab for resources that \
         live only on Teachouse. Fill any tabs you want, leave the rest empty, and upload the \
         whole file.",
        false,
    )?;
    write(sheet, "", false)?;

    write(sheet, "How each tab is laid out", true)?;
    write(sheet, "Row 1 is the column name.", false)?;
    write(
        sheet,
        "Row 2 says whether the column must be filled. \"seller only\" means a legal statement \
         only you can make, so we never fill it in for you.",
        false,
    )?;
    write(
        sheet,
        "Row 3 is a worked example. Leave it and it is ignored on import; type over it and it \
         is treated as your own row.",
        false,
    )?;
    write(sheet, "Row 4 onward is yours.", false)?;
    write(sheet, "", false)?;

    write(sheet, "Status", true)?;
    write(
        sheet,
        &format!(
            "\"{STATUS_DRAFT}\" creates the resource on Teachouse and sends it nowhere. \
             \"{STATUS_LIVE}\" also publishes it to that tab's marketplace. A blank cell reads \
             as \"{STATUS_DRAFT}\"."
        ),
        false,
    )?;
    write(
        sheet,
        &format!(
            "The Teachouse tab accepts only \"{STATUS_DRAFT}\": there is no marketplace for a \
             row there to be live on."
        ),
        false,
    )?;
    write(sheet, "", false)?;

    write(sheet, "File", true)?;
    write(
        sheet,
        "The file name of this row's resource, as it is saved on your computer. Every row on a \
         marketplace tab needs one, draft or live, because buyers need a file to download. Rows \
         on the Teachouse tab don't need one. After you upload the spreadsheet, you attach the \
         files on the import page, and we match them to rows by file name.",
        false,
    )?;
    write(
        sheet,
        "Your files are uploaded to Teachouse and encrypted under your own key. When a live row \
         publishes, the file is sent to the marketplace by your own device, signed in as you. \
         Teachouse never contacts TES or TPT on your behalf.",
        false,
    )?;
    write(sheet, "", false)?;

    write(sheet, "Price and currency", true)?;
    write(
        sheet,
        "A number, or the word \"free\". There is no currency column: the currency follows the \
         marketplace.",
        false,
    )?;
    for tab in TABS {
        let Some(inventory) = tab.inventory else {
            continue;
        };
        write(
            sheet,
            &format!("{} prices in {}.", tab.title, currency_of(inventory)),
            false,
        )?;
    }
    write(sheet, "", false)?;

    write(sheet, "Labels", true)?;
    write(
        sheet,
        "Separate labels with \"; \". If a label has a semicolon in its name, type that \
         semicolon twice. New labels are created for you.",
        false,
    )?;
    write(sheet, "", false)?;

    write(sheet, "Resource ID", true)?;
    write(
        sheet,
        "Leave blank. For now an import only creates new resources, and a row with a resource ID \
         is refused.",
        false,
    )?;
    write(sheet, "", false)?;

    write(sheet, "Saving one tab at a time", true)?;
    write(
        sheet,
        "Upload the whole .xlsx to bring in every tab at once. If you save one tab as a .csv \
         instead, name the file after the tab, like \"TES.csv\" or \"Teachouse.csv\", so we know \
         which tab it is.",
        false,
    )?;
    write(sheet, "", false)?;

    write(sheet, "Limits", true)?;
    write(
        sheet,
        &format!(
            "Up to {} rows across all tabs, and {} MiB per file.",
            tam_limits::import::ROWS_PER_UPLOAD_MAX,
            tam_limits::import::SPREADSHEET_BYTES_MAX.div_euclid(1024 * 1024),
        ),
        false,
    )?;
    write(
        sheet,
        &format!(
            "If any row has a problem, nothing is created. You get a list of what to fix: fix \
             the sheet and upload it again, or import only the rows that passed. An unfinished \
             import is cleared after {} days, along with its files.",
            tam_limits::import::BATCH_EXPIRY_DAYS,
        ),
        false,
    )?;
    write(sheet, "", false)?;

    write(sheet, "Marketplace columns", true)?;
    write(
        sheet,
        "Named as on the marketplace's own form, with the marketplace's internal name in \
         brackets. Columns with a dropdown accept only what the dropdown offers.",
        false,
    )?;
    write(sheet, "", false)?;
    for tab in TABS {
        let held = columns(tab);
        let natives: Vec<&Column> = held
            .iter()
            .filter(|column| matches!(column.cell, Cell::Native(_)))
            .collect();
        if natives.is_empty() {
            continue;
        }
        write(sheet, tab.title, true)?;
        for column in natives {
            let wire = column.wire_name.unwrap_or_default();
            let note = match column.values {
                Values::Closed(_) => "choose from the dropdown",
                // The registry's second closed answer, carried across
                // verbatim: the platform's set is closed, we do not hold its
                // members, and an unlisted value is refused by the platform
                // rather than by us.
                Values::ClosedUncaptured => {
                    "the marketplace accepts only certain values here, and we don't have the list"
                }
                Values::Numeric => "a number",
                Values::Text { .. } => "free text",
            };
            let marker = match column.marker {
                Marker::Required => "required",
                Marker::Optional => "optional",
                Marker::SellerOnly => "seller only",
            };
            write(
                sheet,
                &format!("{} ({wire}) — {marker}; {note}", column.title),
                false,
            )?;
        }
        write(sheet, "", false)?;
    }
    Ok(())
}

/// The denomination one tab prices in, read off the inventory rather than
/// restated, so the workbook cannot disagree with what the create writes.
fn currency_of(inventory: InventoryId) -> &'static str {
    match inventory.currency_rule() {
        CurrencyRule::Fixed(Currency::Gbp) => "GBP",
        CurrencyRule::Fixed(Currency::Usd) => "USD",
        // Neither is reachable from `TABS`, which holds only inventories this
        // build can publish to, and both are stated rather than defaulted: a
        // tab whose currency we do not know says so instead of naming one.
        CurrencyRule::SellerScoped => "the currency your shop is set to",
        CurrencyRule::Unmeasured => "a currency we don't know yet",
    }
}

#[cfg(test)]
mod tests {
    use super::{column_letter, workbook, FIRST_DATA_ROW};
    use crate::import_batch::sheet::{columns, example_row, vocabularies, TABS};

    #[test]
    fn the_workbook_is_a_zip_that_holds_a_worksheet_per_tab() {
        let Ok(bytes) = workbook() else {
            panic!("the template writes");
        };
        assert!(
            bytes.starts_with(b"PK"),
            "an xlsx is a zip, so the bytes open with the local file header signature"
        );
        assert!(
            bytes.len() > 4_096,
            "a workbook of seven tabs is not a few hundred bytes: {} written",
            bytes.len()
        );
    }

    /// The golden fact this test holds is the layout the parser depends on:
    /// three rows before the seller's own, so a refusal citing row four is
    /// citing the first row a seller typed.
    #[test]
    fn the_first_data_row_is_the_fourth_and_the_writer_fills_the_three_above_it() {
        assert_eq!(
            FIRST_DATA_ROW, 4,
            "the header, the marker and the example occupy rows one to three"
        );
        for tab in TABS {
            assert_eq!(
                example_row(tab).len(),
                columns(tab).len(),
                "{}'s example row states one cell per column",
                tab.title
            );
        }
    }

    #[test]
    fn column_letters_follow_excels_own_bijective_base_twenty_six() {
        for (index, expected) in [
            (0_usize, "A"),
            (25, "Z"),
            (26, "AA"),
            (27, "AB"),
            (51, "AZ"),
            (52, "BA"),
            (701, "ZZ"),
            (702, "AAA"),
        ] {
            assert_eq!(
                column_letter(index),
                expected,
                "column {index} is {expected} in Excel's own naming"
            );
        }
    }

    #[test]
    fn the_generated_vocabulary_layout_is_stable_across_calls() {
        let first = vocabularies();
        let second = vocabularies();
        assert_eq!(
            first.iter().map(|(title, _)| title).collect::<Vec<_>>(),
            second.iter().map(|(title, _)| title).collect::<Vec<_>>(),
            "the hidden tab's columns are ordered by the tabs that use them, not by a hash"
        );
    }

    /// Two workbooks generated from one registry are byte-for-byte the same
    /// workbook.
    ///
    /// Asserted on the bytes rather than on the Rust-level ordering, because
    /// the ordering was already stable while the artefact was not: the writer's
    /// default document properties stamp the wall clock into `dcterms:created`
    /// and `dcterms:modified`, so two downloads a second apart differed. This
    /// is the claim a golden test of the header would rest on, so it is held
    /// directly rather than approximated by a proxy that cannot fail.
    #[test]
    fn two_generations_of_the_template_are_the_same_bytes() {
        let (Ok(first), Ok(second)) = (workbook(), workbook()) else {
            panic!("the template writes twice");
        };
        assert_eq!(
            first.len(),
            second.len(),
            "a second download is the same document, so it is the same length"
        );
        assert!(
            first == second,
            "a second download is the same document, so it is the same bytes"
        );
    }
}
