//! Reading a seller's spreadsheet into cells, before anything about those
//! cells is believed.
//!
//! Two formats and one grid model. An `.xlsx` is read through `calamine`'s
//! own cell-at-a-time reader rather than its range reader, and that choice is
//! a defence rather than a style: `Range::from_sparse` allocates a dense
//! `rows * columns` vector from the bounding box of whatever cells it was
//! given, so a two-cell sheet holding A1 and XFD1048576 asks for seventeen
//! billion slots. Streaming cells lets every bound below apply before a cell
//! is kept.
//!
//! A `.csv` is one tab, named by its own filename, read with the same RFC 4180
//! rules `export.rs` writes: comma separated, CRLF or LF between records, a
//! quoted field admitting the separator and a doubled quote inside it. The
//! reader is hand-written for the reason the writer is — this crate holds no
//! CSV library and the shape needed is one grid.
//!
//! An archive is bounded twice before `calamine` opens it, and the two gates
//! catch different things. The first reads the zip central directory and
//! refuses an archive that says it unpacks further than any filled template
//! does; it costs nothing and catches every honest workbook and every naive
//! bomb. The second decompresses every entry through a bounded reader and
//! refuses one that unpacks further than it claimed; it costs one pass over the
//! archive and is what stops an archive that lies. Both are needed: `calamine`
//! reads the shared-string table eagerly inside its own constructor, so by the
//! time this module could inspect a cell the allocation has already happened.
//!
//! Nothing here decides whether a value is admissible. That is
//! [`super::report`]'s job, against the registry. This module answers only:
//! which tabs, which columns, which rows, and what text is in each cell.

use std::collections::BTreeMap;
use std::io::Cursor;

use calamine::{DataType, Reader, Xlsx};

use super::sheet::{tab_of, Tab};
use super::template::FIRST_DATA_ROW;

/// The longest single cell this reader will keep, in bytes.
///
/// `crate::import::COPY_MAX`'s own reasoning, applied at the other end of the
/// same problem: a seller's title and body are free text that no vocabulary can
/// bound, so they are bounded where they enter. Generous against any real
/// description and small against a payload.
pub const CELL_BYTES_MAX: usize = 64 * 1024;

/// How many non-empty cells one upload may carry, across every tab.
///
/// The row ceiling bounds rows and this bounds the product of rows and columns,
/// which is the quantity a crafted file actually inflates. Five hundred rows of
/// the widest tab is a few tens of thousands, so this sits an order above any
/// workbook this template produces and refuses well before a dense structure
/// costs anything.
pub const CELLS_MAX: usize = 200_000;

/// How far right a tab is read.
///
/// The widest tab this template writes is under forty columns. A file with
/// hundreds is not a filled template; it is a pasted grid or a crafted one, and
/// reading it would let a single row carry an unbounded number of cells.
pub const COLUMNS_MAX: usize = 128;

/// How far down a tab is read, as a zero-based row index.
///
/// This bounds the row *index* rather than the row *count*, and the two are not
/// the same defence. `CELLS_MAX` bounds how many cells are kept and
/// `ROWS_PER_UPLOAD_MAX` bounds how many rows an upload carries, but a sheet
/// holding two cells — one at A1 and one at row two billion — passes both while
/// naming a range two billion long. `grid_of` walks the range rather than the
/// cells, so without this the walk is the attack: a sub-kilobyte file costs an
/// unbounded, non-yielding loop on whichever worker thread served the request.
///
/// A cell reference's row is accumulated into a plain `u32` by calamine's own
/// parser with no comparison against Excel's real row limit, so the bound has
/// to be applied here. Checked as each cell arrives, before it is kept, because
/// a declared `<dimension>` is a claim the file makes about itself and the zip
/// sizes already taught us what those are worth.
///
/// The three rows above the data are the header, the marker and the example.
pub const ROW_INDEX_MAX: usize = 2 + tam_limits::import::ROWS_PER_UPLOAD_MAX;

/// The largest any one entry inside an `.xlsx` may declare it expands to.
///
/// An xlsx is a zip, and the two files that make one large are the sheet XML
/// and the shared-string table. A five-hundred-row workbook's sheet is under a
/// megabyte of XML; this is two orders above that and far below what a box
/// notices.
pub const ENTRY_UNCOMPRESSED_BYTES_MAX: u64 = 64 * 1024 * 1024;

/// The largest an `.xlsx` may declare it expands to in total.
pub const ARCHIVE_UNCOMPRESSED_BYTES_MAX: u64 = 128 * 1024 * 1024;

/// One tab, as cells rather than as meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    /// The tab this grid came off.
    pub tab: Tab,
    /// The header cells in column order, trimmed, as the file holds them.
    pub header: Vec<String>,
    /// The rows the seller typed, in file order.
    pub rows: Vec<GridRow>,
}

/// One row of one tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridRow {
    /// The seller's own spreadsheet row number, counting from one, so a
    /// refusal cites the number they see in the margin rather than an index
    /// into a parse.
    pub number: u32,
    /// One cell per header column, trimmed, blank where the file held nothing.
    pub cells: Vec<String>,
}

/// Why a file could not be read into a grid at all.
///
/// Distinct from a row's own refusal: everything here refuses the upload
/// before any row exists to name, so none of it can be reported per row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Malformed {
    /// Neither a workbook nor a comma-separated file.
    UnknownFormat,
    /// The bytes are an xlsx and could not be opened.
    ///
    /// `detail` is the upstream crate's own words — calamine's, `zip`'s, or the
    /// standard library's — and is deliberately not part of what this refusal
    /// says. Every other message in this module is written here, in this
    /// project's voice, and forwarding a third-party string to a seller because
    /// it happened to be at hand is the one place that would stop being true.
    /// [`Malformed::upstream`] hands it to the caller, which surfaces it only
    /// where the deployment's disclosure setting allows internals.
    Unreadable { detail: String },
    /// An xlsx whose own central directory declares an expansion no filled
    /// template needs.
    DeclaresTooLarge { declared: u64, limit: u64 },
    /// An xlsx that unpacks further than it declared, caught by reading it.
    ///
    /// Carries the ceiling and not the true size, because the reader stops at
    /// the ceiling and never learns it: reporting a number here would be
    /// reporting the bound as though it were a measurement.
    ExpandsTooFar { limit: u64 },
    /// The file names no tab this workbook has.
    NoKnownTab { held: Vec<String> },
    /// A `.csv` whose filename does not name a tab.
    UnnamedCsv { stem: String },
    /// A tab whose header row is empty, so no column can be addressed.
    HeaderMissing { sheet: String },
    /// A tab whose header does not carry a column the template writes.
    ///
    /// A structural refusal rather than a refusal of every row, because a
    /// five-hundred-line report saying one thing five hundred times is not a
    /// report, and no batch should be written for a sheet whose shape is wrong.
    ColumnMissing { sheet: String, column: String },
    /// A tab carrying a column the template does not write.
    ///
    /// Refused rather than ignored: a seller who added a column and filled it
    /// in expected it to matter, and silently dropping it is the failure they
    /// would find out about from their listings.
    ColumnUnknown { sheet: String, column: String },
    /// More rows than one upload may carry.
    TooManyRows { rows: usize, limit: usize },
    /// More cells than one upload may carry.
    TooManyCells { limit: usize },
    /// A tab reaching further right than any template does.
    TooManyColumns { sheet: String, limit: usize },
    /// A tab naming a row further down than any upload may carry.
    ///
    /// Distinct from `TooManyRows`, which counts the rows an upload holds. This
    /// one refuses a row *position*, which a sheet can name without carrying
    /// the rows in between, and which is what makes the walk expensive.
    RowOutOfRange {
        sheet: String,
        row: u32,
        limit: usize,
    },
    /// One cell longer than any cell may be.
    CellTooLong {
        sheet: String,
        row: u32,
        column: u32,
        bytes: usize,
        limit: usize,
    },
    /// Every tab was empty, so the upload states nothing.
    NoRows,
}

impl core::fmt::Display for Malformed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownFormat => f.write_str("Upload an .xlsx or .csv file."),
            Self::Unreadable { .. } => f.write_str(
                "We can't open this file. If it opens in your spreadsheet program, save it again \
                 as .xlsx and upload that.",
            ),
            Self::DeclaresTooLarge { declared, limit } => write!(
                f,
                "This file is too big: it opens to {declared} bytes, and an import takes at most \
                 {limit}."
            ),
            Self::ExpandsTooFar { limit } => write!(
                f,
                "This file is too big: it opens to more than {limit} bytes."
            ),
            Self::NoKnownTab { held } => write!(
                f,
                "None of this file's tabs match the template. It has {}.",
                held.join(", ")
            ),
            Self::UnnamedCsv { stem } => write!(
                f,
                "Name your .csv after a template tab. \"{stem}\" is not one."
            ),
            Self::HeaderMissing { sheet } => {
                write!(f, "The {sheet} tab needs column names in its first row.")
            }
            Self::ColumnMissing { sheet, column } => write!(
                f,
                "The {sheet} tab has no \"{column}\" column. Download the template again and \
                 copy your rows into it."
            ),
            Self::ColumnUnknown { sheet, column } => write!(
                f,
                "The {sheet} tab has a \"{column}\" column that the import doesn't use. Remove \
                 it and upload again."
            ),
            Self::TooManyRows { rows, limit } => write!(
                f,
                "This file has {rows} rows. You can import up to {limit} at once."
            ),
            Self::TooManyCells { limit } => write!(
                f,
                "This file has more than {limit} filled cells, which is too many for one import."
            ),
            Self::TooManyColumns { sheet, limit } => write!(
                f,
                "The {sheet} tab goes past column {limit}. Delete anything beyond it."
            ),
            Self::RowOutOfRange { sheet, row, limit } => write!(
                f,
                "The {sheet} tab has something in row {row}, but the import stops at row \
                 {limit}. Delete everything below that row and upload again."
            ),
            Self::CellTooLong {
                sheet,
                row,
                column,
                bytes,
                limit,
            } => write!(
                f,
                "In the {sheet} tab, row {row}, column {column} is too long: {bytes} bytes, and \
                 a cell holds at most {limit}."
            ),
            Self::NoRows => f.write_str("Every tab in this file is empty."),
        }
    }
}

impl Malformed {
    /// The upstream crate's own words, where this refusal wraps one.
    ///
    /// Separate from [`core::fmt::Display`] so that the seller-facing sentence
    /// and the diagnostic string cannot be confused for one another: the first
    /// is always shown, and this is shown only where the deployment discloses
    /// internals.
    #[must_use]
    pub fn upstream(&self) -> Option<&str> {
        match self {
            Self::Unreadable { detail } => Some(detail),
            Self::UnknownFormat
            | Self::DeclaresTooLarge { .. }
            | Self::ExpandsTooFar { .. }
            | Self::NoKnownTab { .. }
            | Self::UnnamedCsv { .. }
            | Self::HeaderMissing { .. }
            | Self::ColumnMissing { .. }
            | Self::ColumnUnknown { .. }
            | Self::TooManyRows { .. }
            | Self::TooManyCells { .. }
            | Self::TooManyColumns { .. }
            | Self::RowOutOfRange { .. }
            | Self::CellTooLong { .. }
            | Self::NoRows => None,
        }
    }
}

/// Every grid a file holds, in the workbook's own tab order.
///
/// `source_name` is the uploaded filename, which is what names the tab of a
/// `.csv` and is otherwise only recorded.
pub fn grids(source_name: &str, bytes: &[u8]) -> Result<Vec<Grid>, Malformed> {
    let grids = if is_zip(bytes) {
        workbook_grids(bytes)?
    } else {
        vec![csv_grid(source_name, bytes)?]
    };
    // Emptiness is not decided here. A sheet whose header is wrong holds no
    // readable rows either, and telling a seller their file is empty when the
    // real answer is that a column is missing sends them to fix the wrong
    // thing. `super::report::report` raises `NoRows` after it has checked the
    // shape.
    let rows: usize = grids.iter().map(|grid| grid.rows.len()).sum();
    let limit = tam_limits::import::ROWS_PER_UPLOAD_MAX;
    if rows > limit {
        return Err(Malformed::TooManyRows { rows, limit });
    }
    Ok(grids)
}

/// Whether the bytes open with a zip local-file header, which is what an xlsx
/// is. Sniffed rather than taken from the filename: a browser's own content
/// type and a seller's own extension are both things this route is told rather
/// than things it observes.
fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

// ------------------------------------------------------------------ xlsx

fn workbook_grids(bytes: &[u8]) -> Result<Vec<Grid>, Malformed> {
    refuse_a_bomb(bytes)?;
    refuse_an_expansion(
        bytes,
        ENTRY_UNCOMPRESSED_BYTES_MAX,
        ARCHIVE_UNCOMPRESSED_BYTES_MAX,
    )?;
    let mut book: Xlsx<_> =
        Xlsx::new(Cursor::new(bytes)).map_err(|error| Malformed::Unreadable {
            detail: error.to_string(),
        })?;
    let names = book.sheet_names();
    let known: Vec<(String, Tab)> = names
        .iter()
        .filter_map(|name| tab_of(name).map(|tab| (name.clone(), tab)))
        .collect();
    if known.is_empty() {
        return Err(Malformed::NoKnownTab { held: names });
    }

    let mut kept = 0_usize;
    let mut grids = Vec::with_capacity(known.len());
    for (name, tab) in known {
        // Cell at a time, so the bounds below apply before anything is kept.
        // The range reader would allocate the bounding box first.
        let mut reader =
            book.worksheet_cells_reader(&name)
                .map_err(|error| Malformed::Unreadable {
                    detail: error.to_string(),
                })?;
        let mut cells: BTreeMap<(u32, u32), String> = BTreeMap::new();
        loop {
            let next = reader.next_cell().map_err(|error| Malformed::Unreadable {
                detail: error.to_string(),
            })?;
            let Some(cell) = next else {
                break;
            };
            let (row, column) = cell.get_position();
            if usize::try_from(column).unwrap_or(usize::MAX) >= COLUMNS_MAX {
                return Err(Malformed::TooManyColumns {
                    sheet: name.clone(),
                    limit: COLUMNS_MAX,
                });
            }
            // Before the cell is kept, and before anything walks the range it
            // implies. A single cell far down the sheet is what makes
            // `grid_of`'s walk unbounded, and it costs nothing to carry here.
            if usize::try_from(row).unwrap_or(usize::MAX) > ROW_INDEX_MAX {
                return Err(Malformed::RowOutOfRange {
                    sheet: name.clone(),
                    row: row.saturating_add(1),
                    limit: ROW_INDEX_MAX.saturating_add(1),
                });
            }
            let Some(text) = cell.get_value().as_string() else {
                continue;
            };
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.len() > CELL_BYTES_MAX {
                return Err(Malformed::CellTooLong {
                    sheet: name.clone(),
                    // Both counted the way a seller reads them, from one.
                    row: row.saturating_add(1),
                    column: column.saturating_add(1),
                    bytes: trimmed.len(),
                    limit: CELL_BYTES_MAX,
                });
            }
            kept = kept.saturating_add(1);
            if kept > CELLS_MAX {
                return Err(Malformed::TooManyCells { limit: CELLS_MAX });
            }
            cells.insert((row, column), trimmed.to_owned());
        }
        grids.push(grid_of(tab, &name, &cells)?);
    }
    Ok(grids)
}

/// A tab's sparse cells as a grid, keyed on the layout the template writes.
fn grid_of(tab: Tab, name: &str, cells: &BTreeMap<(u32, u32), String>) -> Result<Grid, Malformed> {
    let width = cells
        .keys()
        .map(|(_, column)| usize::try_from(*column).unwrap_or(0).saturating_add(1))
        .max()
        .unwrap_or(0);
    let header: Vec<String> = (0..width)
        .map(|column| {
            let at = u32::try_from(column).unwrap_or(u32::MAX);
            cells.get(&(0, at)).cloned().unwrap_or_default()
        })
        .collect();
    if header.iter().all(String::is_empty) {
        // An entirely empty tab is not a header fault; it is a tab the seller
        // did not fill, and the upload is allowed to hold several.
        if cells.is_empty() {
            return Ok(Grid {
                tab,
                header,
                rows: Vec::new(),
            });
        }
        return Err(Malformed::HeaderMissing {
            sheet: name.to_owned(),
        });
    }

    // The seller's own rows start below the header, the marker and the
    // example, and their numbers are what a refusal cites.
    let first = FIRST_DATA_ROW.saturating_sub(1);
    let mut rows = Vec::new();
    let last = cells.keys().map(|(row, _)| *row).max().unwrap_or(0);
    for row in first..=last {
        let values: Vec<String> = (0..width)
            .map(|column| {
                let at = u32::try_from(column).unwrap_or(u32::MAX);
                cells.get(&(row, at)).cloned().unwrap_or_default()
            })
            .collect();
        if values.iter().all(String::is_empty) {
            continue;
        }
        rows.push(GridRow {
            number: row.saturating_add(1),
            cells: values,
        });
    }
    Ok(Grid { tab, header, rows })
}

/// The cheap gate: refuses a workbook that says it unpacks further than any
/// filled template does, read off the zip's own central directory without
/// decompressing a byte.
///
/// It catches an archive that declares its expansion honestly, which is every
/// workbook a spreadsheet program writes and every naive bomb, and it does so
/// for the cost of a backwards scan. An archive that under-declares passes here
/// and is caught by [`refuse_an_expansion`] below, which is the gate that
/// actually reads. This one exists so that the common refusal is cheap.
fn refuse_a_bomb(bytes: &[u8]) -> Result<(), Malformed> {
    let mut declared = 0_u64;
    for entry in central_directory(bytes) {
        if entry > ENTRY_UNCOMPRESSED_BYTES_MAX {
            return Err(Malformed::DeclaresTooLarge {
                declared: entry,
                limit: ENTRY_UNCOMPRESSED_BYTES_MAX,
            });
        }
        declared = declared.saturating_add(entry);
    }
    if declared > ARCHIVE_UNCOMPRESSED_BYTES_MAX {
        return Err(Malformed::DeclaresTooLarge {
            declared,
            limit: ARCHIVE_UNCOMPRESSED_BYTES_MAX,
        });
    }
    Ok(())
}

/// The gate that reads: decompresses every entry through a bounded reader and
/// refuses one that unpacks further than it claimed.
///
/// This is the half [`refuse_a_bomb`] cannot do. `calamine` reads the
/// shared-string table eagerly inside `Xlsx::new`, and the zip reader beneath
/// it bounds the *compressed* input it feeds the decompressor rather than the
/// output it takes back, so an entry that declares a kilobyte and delivers
/// gigabytes allocates them before any code here runs. Reading every entry
/// first, into a sink rather than into memory, is what makes the ceiling true
/// of the archive rather than of its own account of itself.
///
/// `zip8` is the `zip` crate under an alias, at `calamine`'s own version and
/// feature set, so the entries bounded here are exactly the entries it will
/// decompress; a reader that parsed the archive differently would bound a
/// different set. The alias exists because this crate also carries `zip 2` as a
/// dev-dependency, and two candidates for one name will not link.
///
/// The bounds are parameters rather than the constants directly, so a test can
/// prove the mechanism against a small archive instead of building a large one.
/// The one caller passes the constants.
fn refuse_an_expansion(bytes: &[u8], entry_max: u64, archive_max: u64) -> Result<(), Malformed> {
    let mut archive = zip8::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|error| {
        Malformed::Unreadable {
            detail: error.to_string(),
        }
    })?;
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| Malformed::Unreadable {
                detail: error.to_string(),
            })?;
        // Into a sink, so the pass costs a buffer rather than the archive: the
        // question is how far this unpacks, not what it unpacks to.
        let read = std::io::copy(
            &mut std::io::Read::take(&mut entry, entry_max.saturating_add(1)),
            &mut std::io::sink(),
        )
        .map_err(|error| Malformed::Unreadable {
            detail: error.to_string(),
        })?;
        if read > entry_max {
            return Err(Malformed::ExpandsTooFar { limit: entry_max });
        }
        total = total.saturating_add(read);
        if total > archive_max {
            return Err(Malformed::ExpandsTooFar { limit: archive_max });
        }
    }
    Ok(())
}

/// Every entry's declared uncompressed length, from the zip central directory.
///
/// Hand-read rather than taken from a zip library, because the only fact needed
/// is one `u32` per entry and this crate holds no zip reader. A file whose
/// central directory cannot be located yields nothing, which lets `calamine`
/// produce the refusal: this function's job is to refuse a bomb, not to decide
/// whether an archive is well formed.
///
/// A zip64 entry, which declares `0xFFFF_FFFF` here and states its real length
/// in an extra field, is reported as the ceiling itself rather than parsed: no
/// filled template needs zip64, so reporting it as over the ceiling is the
/// fail-closed reading.
fn central_directory(bytes: &[u8]) -> Vec<u64> {
    const END_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    const ENTRY_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
    const END_RECORD_LEN: usize = 22;
    const ENTRY_FIXED_LEN: usize = 46;

    let Some(end) = bytes
        .windows(END_SIGNATURE.len())
        .rposition(|window| window == END_SIGNATURE)
    else {
        return Vec::new();
    };
    let Some(record) = bytes.get(end..end.saturating_add(END_RECORD_LEN)) else {
        return Vec::new();
    };
    let Some(mut at) = le_u32(record, 16).map(|offset| usize::try_from(offset).unwrap_or(0)) else {
        return Vec::new();
    };

    let mut declared = Vec::new();
    while let Some(entry) = bytes.get(at..at.saturating_add(ENTRY_FIXED_LEN)) {
        if entry.get(..4) != Some(&ENTRY_SIGNATURE) {
            break;
        }
        let (Some(size), Some(name_len), Some(extra_len), Some(comment_len)) = (
            le_u32(entry, 24),
            le_u16(entry, 28),
            le_u16(entry, 30),
            le_u16(entry, 32),
        ) else {
            break;
        };
        declared.push(if size == u32::MAX {
            ENTRY_UNCOMPRESSED_BYTES_MAX.saturating_add(1)
        } else {
            u64::from(size)
        });
        at = at
            .saturating_add(ENTRY_FIXED_LEN)
            .saturating_add(usize::from(name_len))
            .saturating_add(usize::from(extra_len))
            .saturating_add(usize::from(comment_len));
    }
    declared
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.saturating_add(4))?;
    Some(u32::from_le_bytes([
        *slice.first()?,
        *slice.get(1)?,
        *slice.get(2)?,
        *slice.get(3)?,
    ]))
}

fn le_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at.saturating_add(2))?;
    Some(u16::from_le_bytes([*slice.first()?, *slice.get(1)?]))
}

// ------------------------------------------------------------------- csv

/// One comma-separated file as one tab.
///
/// The tab is named by the filename, because a `.csv` carries no tab name and
/// guessing one from the columns would guess wrong the first time two tabs
/// shared a header prefix. `TES.csv`, `tes gb.csv` and `TES (1).csv` all
/// name the TES tab; anything else is refused by name with the titles that
/// would have worked.
fn csv_grid(source_name: &str, bytes: &[u8]) -> Result<Grid, Malformed> {
    let text = core::str::from_utf8(bytes).map_err(|_| Malformed::UnknownFormat)?;
    let stem = csv_stem(source_name);
    let Some(tab) = super::sheet::TABS
        .into_iter()
        .find(|tab| tab.title.eq_ignore_ascii_case(&stem))
    else {
        return Err(Malformed::UnnamedCsv { stem });
    };

    let records = records(text);
    let width = records.iter().map(Vec::len).max().unwrap_or(0);
    if width > COLUMNS_MAX {
        return Err(Malformed::TooManyColumns {
            sheet: tab.title.to_owned(),
            limit: COLUMNS_MAX,
        });
    }
    let mut kept = 0_usize;
    for (index, record) in records.iter().enumerate() {
        for (column, cell) in record.iter().enumerate() {
            if cell.is_empty() {
                continue;
            }
            if cell.len() > CELL_BYTES_MAX {
                return Err(Malformed::CellTooLong {
                    sheet: tab.title.to_owned(),
                    row: u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1),
                    column: u32::try_from(column).unwrap_or(u32::MAX).saturating_add(1),
                    bytes: cell.len(),
                    limit: CELL_BYTES_MAX,
                });
            }
            kept = kept.saturating_add(1);
            if kept > CELLS_MAX {
                return Err(Malformed::TooManyCells { limit: CELLS_MAX });
            }
        }
    }

    let widened = |record: Option<&Vec<String>>| -> Vec<String> {
        let mut cells = record.cloned().unwrap_or_default();
        cells.resize(width, String::new());
        cells
    };
    let header = widened(records.first());
    if header.iter().all(String::is_empty) {
        if records
            .iter()
            .all(|record| record.iter().all(String::is_empty))
        {
            return Ok(Grid {
                tab,
                header,
                rows: Vec::new(),
            });
        }
        return Err(Malformed::HeaderMissing {
            sheet: tab.title.to_owned(),
        });
    }

    let first = usize::try_from(FIRST_DATA_ROW.saturating_sub(1)).unwrap_or(0);
    let rows = records
        .iter()
        .enumerate()
        .skip(first)
        .filter_map(|(index, record)| {
            let cells = widened(Some(record));
            if cells.iter().all(String::is_empty) {
                return None;
            }
            Some(GridRow {
                number: u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1),
                cells,
            })
        })
        .collect();
    Ok(Grid { tab, header, rows })
}

/// The filename without its extension and without a browser's own
/// disambiguating suffix, which is what a seller sees when they save the same
/// tab twice.
fn csv_stem(source_name: &str) -> String {
    let name = source_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(source_name);
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let trimmed = stem
        .rsplit_once(" (")
        .filter(|(_, suffix)| {
            suffix.ends_with(')')
                && suffix
                    .trim_end_matches(')')
                    .chars()
                    .all(|character| character.is_ascii_digit())
        })
        .map_or(stem, |(head, _)| head);
    trimmed.trim().to_owned()
}

/// One document as records of fields, RFC 4180: comma separated, CRLF or LF
/// between records, a quoted field admitting both and a doubled quote inside
/// it. The mirror of `export.rs`'s `field`, so a catalogue exported by this API
/// reads back through it.
fn records(text: &str) -> Vec<Vec<String>> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut characters = text.chars().peekable();
    let mut started = false;

    while let Some(character) = characters.next() {
        started = true;
        if quoted {
            if character == '"' {
                if characters.peek() == Some(&'"') {
                    let _consumed = characters.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(character);
            }
            continue;
        }
        match character {
            '"' if field.is_empty() => quoted = true,
            ',' => record.push(core::mem::take(&mut field).trim().to_owned()),
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    let _consumed = characters.next();
                }
                record.push(core::mem::take(&mut field).trim().to_owned());
                records.push(core::mem::take(&mut record));
            }
            '\n' => {
                record.push(core::mem::take(&mut field).trim().to_owned());
                records.push(core::mem::take(&mut record));
            }
            _ => field.push(character),
        }
    }
    if started && (!field.is_empty() || !record.is_empty()) {
        record.push(field.trim().to_owned());
        records.push(record);
    }
    records
}

#[cfg(test)]
mod tests {
    use super::{
        csv_stem, grids, records, Malformed, ARCHIVE_UNCOMPRESSED_BYTES_MAX, CELL_BYTES_MAX,
    };
    use crate::import_batch::sheet::{columns, example_row, Cell};
    use crate::import_batch::template::workbook;

    /// An unfilled template states no rows, which is the one thing it can say:
    /// its own example row sits above the first data row, so a seller who
    /// uploads the template untouched is told it is empty rather than having
    /// the example imported as a resource.
    #[test]
    fn the_unfilled_template_holds_no_rows() {
        let Ok(bytes) = workbook() else {
            panic!("the template writes");
        };
        let Ok(read) = grids("teachouse-import-template.xlsx", &bytes) else {
            panic!("the template this repository writes parses");
        };
        assert_eq!(
            crate::import_batch::report::report(&read).err(),
            Some(Malformed::NoRows),
            "the example row is above the data region, so it is never a resource"
        );
    }

    /// A workbook written with the template's own headers, filled with one row
    /// on two tabs, read back through this module.
    ///
    /// The round trip is the property this feature rests on: a workbook the
    /// seller downloads has to be a workbook this parser recognises, and both
    /// sides are derived from one column model, so a native field added to the
    /// registry moves the writer and the reader in the same edit or neither.
    #[test]
    fn a_filled_workbook_reads_back_as_the_tabs_and_rows_it_holds() {
        let Ok(bytes) = filled_workbook(&[
            ("TES", "Fractions worksheet", "2.50"),
            ("Teachouse", "A free planning grid", "free"),
        ]) else {
            panic!("the fixture workbook writes");
        };
        let Ok(read) = grids("filled.xlsx", &bytes) else {
            panic!("a filled workbook reads");
        };
        assert_eq!(read.len(), 3, "every grid tab of the template is present");
        let filled: Vec<(&str, usize, u32)> = read
            .iter()
            .filter(|grid| !grid.rows.is_empty())
            .map(|grid| {
                (
                    grid.tab.title,
                    grid.rows.len(),
                    grid.rows.first().map_or(0, |row| row.number),
                )
            })
            .collect();
        assert_eq!(
            filled,
            vec![("Teachouse", 1, 4), ("TES", 1, 4)],
            "each filled tab holds its one row, numbered as the seller reads it"
        );
        let Some(grid) = read.iter().find(|grid| grid.tab.title == "TES") else {
            panic!("the TES tab is present");
        };
        let held = columns(grid.tab);
        let Some(title_at) = held.iter().position(|column| column.cell == Cell::Title) else {
            panic!("every tab has a title column");
        };
        assert_eq!(
            grid.rows.first().and_then(|row| row.cells.get(title_at)),
            Some(&"Fractions worksheet".to_owned()),
            "a cell reads back under the column its header names"
        );
    }

    /// A workbook laid out exactly as the template writes one, with a single
    /// row filled on each named tab. Written with the same crate the template
    /// generator uses, so the fixture cannot drift from the real file's shape.
    fn filled_workbook(rows: &[(&str, &str, &str)]) -> Result<Vec<u8>, rust_xlsxwriter::XlsxError> {
        use rust_xlsxwriter::{Workbook, Worksheet};
        let mut book = Workbook::new();
        for tab in crate::import_batch::sheet::TABS {
            let mut sheet = Worksheet::new();
            sheet.set_name(tab.title)?;
            let held = columns(tab);
            for (index, column) in held.iter().enumerate() {
                let at = u16::try_from(index).unwrap_or(u16::MAX);
                sheet.write_string(0, at, &column.title)?;
                sheet.write_string(1, at, column.marker.as_str())?;
                if let Some(cell) = example_row(tab).get(index) {
                    sheet.write_string(2, at, cell)?;
                }
            }
            if let Some((_, title, price)) = rows.iter().find(|(name, _, _)| *name == tab.title) {
                for (index, column) in held.iter().enumerate() {
                    let at = u16::try_from(index).unwrap_or(u16::MAX);
                    let value = match column.cell {
                        Cell::Title => (*title).to_owned(),
                        Cell::Price => (*price).to_owned(),
                        Cell::Status => "draft".to_owned(),
                        Cell::ResourceId
                        | Cell::Description
                        | Cell::Labels
                        | Cell::File
                        | Cell::Currency
                        | Cell::Native(_) => String::new(),
                    };
                    if !value.is_empty() {
                        sheet.write_string(3, at, &value)?;
                    }
                }
            }
            book.push_worksheet(sheet);
        }
        book.save_to_buffer()
    }

    fn filled(tab_title: &str, values: &[(Cell, &str)]) -> String {
        let Some(tab) = crate::import_batch::sheet::tab_of(tab_title) else {
            panic!("{tab_title} is a tab");
        };
        let held = columns(tab);
        let header: Vec<String> = held.iter().map(|column| column.title.clone()).collect();
        let marker: Vec<String> = held
            .iter()
            .map(|column| column.marker.as_str().to_owned())
            .collect();
        let example = example_row(tab);
        let row: Vec<String> = held
            .iter()
            .map(|column| {
                values
                    .iter()
                    .find(|(cell, _)| *cell == column.cell)
                    .map_or(String::new(), |(_, value)| (*value).to_owned())
            })
            .collect();
        [header, marker, example, row]
            .iter()
            .map(|record| record.join(","))
            .collect::<Vec<_>>()
            .join("\r\n")
    }

    #[test]
    fn a_csv_is_the_tab_its_filename_names_and_its_rows_start_at_four() {
        let document = filled(
            "TES",
            &[(Cell::Title, "A worksheet"), (Cell::Price, "2.50")],
        );
        let Ok(read) = grids("TES.csv", document.as_bytes()) else {
            panic!("a filled TES csv reads");
        };
        assert_eq!(read.len(), 1, "a csv is one tab");
        let Some(grid) = read.first() else {
            panic!("the one grid is present");
        };
        assert_eq!(grid.tab.title, "TES");
        assert_eq!(grid.rows.len(), 1, "the example row is not the seller's");
        let Some(row) = grid.rows.first() else {
            panic!("the one row is present");
        };
        assert_eq!(
            row.number, 4,
            "the first row a seller types is row four, and that is the number a refusal cites"
        );
    }

    #[test]
    fn a_csv_whose_filename_names_no_tab_is_refused_by_that_name() {
        let document = filled("TES", &[(Cell::Title, "A worksheet")]);
        let refused = grids("my resources.csv", document.as_bytes());
        assert_eq!(
            refused.err(),
            Some(Malformed::UnnamedCsv {
                stem: "my resources".to_owned()
            }),
            "the refusal names what it read rather than saying the format is wrong"
        );
    }

    #[test]
    fn a_browsers_duplicate_suffix_still_names_its_tab() {
        for name in ["TES.csv", "tes gb.csv", "TES (1).csv", "/tmp/TES.csv"] {
            assert_eq!(
                csv_stem(name),
                if name.contains("tes gb") {
                    "tes gb"
                } else {
                    "TES"
                },
                "{name} names the TES tab"
            );
        }
    }

    #[test]
    fn the_export_conventions_survive_the_round_trip() {
        // `export.rs` doubles a semicolon inside a label and quotes a field
        // holding a comma, so both have to read back verbatim.
        let read = records("a,\"b,c\",\"d\"\"e\"\r\nf,g,h\r\n");
        assert_eq!(
            read,
            vec![
                vec!["a".to_owned(), "b,c".to_owned(), "d\"e".to_owned()],
                vec!["f".to_owned(), "g".to_owned(), "h".to_owned()],
            ],
            "a quoted field admits the separator, and a doubled quote is one quote"
        );
    }

    #[test]
    fn a_record_ending_without_a_newline_is_still_a_record() {
        assert_eq!(
            records("a,b"),
            vec![vec!["a".to_owned(), "b".to_owned()]],
            "a file whose last line has no terminator holds that line"
        );
    }

    #[test]
    fn an_over_long_cell_is_refused_naming_the_cell() {
        let long = "x".repeat(CELL_BYTES_MAX + 1);
        let document = filled("TES", &[(Cell::Title, "t")]);
        let with_long = format!("{document}\r\n{long}");
        let refused = grids("TES.csv", with_long.as_bytes());
        assert!(
            matches!(refused, Err(Malformed::CellTooLong { .. })),
            "a cell past the ceiling is refused by position, not truncated: {refused:?}"
        );
    }

    #[test]
    fn an_empty_upload_states_nothing_and_says_so() {
        let Ok(read) = grids("TES.csv", b"") else {
            panic!("an empty file is read as an empty tab rather than a parse fault");
        };
        assert_eq!(
            crate::import_batch::report::report(&read).err(),
            Some(Malformed::NoRows),
            "an empty file is refused rather than written as a batch of no rows"
        );
    }

    /// The ceiling is a claim about the archive's own central directory, so a
    /// workbook this repository writes must sit well under it — otherwise the
    /// bound would refuse the template it ships.
    #[test]
    fn the_generated_template_sits_far_under_the_expansion_ceiling() {
        let Ok(bytes) = workbook() else {
            panic!("the template writes");
        };
        let declared: u64 = super::central_directory(&bytes).iter().sum();
        assert!(
            declared > 0,
            "the central directory of a real xlsx is readable"
        );
        assert!(
            declared * 8 < ARCHIVE_UNCOMPRESSED_BYTES_MAX,
            "the shipped template unpacks to {declared} bytes, which leaves no headroom under \
             the ceiling"
        );
    }

    /// The workbook this repository ships passes the gate that reads, under the
    /// real ceilings. Without this the bound could be set anywhere and the only
    /// evidence would be that bombs are refused.
    #[test]
    fn the_generated_template_passes_the_bounded_read() {
        let Ok(bytes) = workbook() else {
            panic!("the template writes");
        };
        assert_eq!(
            super::refuse_an_expansion(
                &bytes,
                super::ENTRY_UNCOMPRESSED_BYTES_MAX,
                super::ARCHIVE_UNCOMPRESSED_BYTES_MAX,
            ),
            Ok(()),
            "the shipped template is read, not refused, by its own ceiling"
        );
    }

    /// The property the cheap gate cannot have: an archive that lies about how
    /// far it unpacks is still refused, because the second gate reads it.
    ///
    /// Built by taking a real workbook and rewriting every declared
    /// uncompressed length to one byte, which is exactly what a crafted archive
    /// does. The first gate then sees an archive claiming to unpack to nothing
    /// and passes it; the assertion is that the upload is refused anyway, and
    /// that the refusal is not the first gate's.
    #[test]
    fn an_archive_that_under_declares_its_sizes_is_still_refused() {
        let Ok(bytes) = workbook() else {
            panic!("the template writes");
        };
        let lying = understate_declarations(&bytes);

        assert_eq!(
            super::refuse_a_bomb(&lying),
            Ok(()),
            "the cheap gate reads the declaration, and this archive declares nothing; if this \
             ever fails the test below stops proving what it claims"
        );

        // A ceiling under the real workbook's own expansion, so the mechanism
        // is proved without building a multi-gigabyte fixture. The ceilings the
        // route passes are held against the shipped template by the test above.
        let refused = super::refuse_an_expansion(&lying, 1_024, 1_024);
        assert!(
            matches!(refused, Err(Malformed::ExpandsTooFar { .. })),
            "an archive is bounded by what it unpacks to, not by what it says: {refused:?}"
        );
    }

    /// Rewrites every declared uncompressed length in a zip to one byte, in
    /// both the central directory and the local file headers, leaving the
    /// compressed data untouched. What a crafted archive does, done to a real
    /// one.
    fn understate_declarations(bytes: &[u8]) -> Vec<u8> {
        const CENTRAL: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
        const LOCAL: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
        const CENTRAL_UNCOMPRESSED_AT: usize = 24;
        const LOCAL_UNCOMPRESSED_AT: usize = 22;

        let mut patched = bytes.to_vec();
        let mut at = 0_usize;
        while at + 4 <= patched.len() {
            let signature = patched.get(at..at + 4).unwrap_or_default();
            let field = if signature == CENTRAL {
                Some(CENTRAL_UNCOMPRESSED_AT)
            } else if signature == LOCAL {
                Some(LOCAL_UNCOMPRESSED_AT)
            } else {
                None
            };
            if let Some(offset) = field {
                let start = at.saturating_add(offset);
                if let Some(slot) = patched.get_mut(start..start.saturating_add(4)) {
                    slot.copy_from_slice(&1_u32.to_le_bytes());
                }
            }
            at = at.saturating_add(1);
        }
        patched
    }

    /// B1: a sheet naming a row two billion down is refused, and refused fast.
    ///
    /// The severity is the timing assertion, not the refusal. Before the row
    /// bound existed this file passed every size, count and byte gate — two
    /// cells, one column, a few hundred bytes — and then `grid_of` walked the
    /// range those two cells implied, on the thread serving the request, with
    /// nothing in the loop yielding or checking for a disconnected client. The
    /// refusal proves the bound is applied; the elapsed time proves it is
    /// applied before the walk rather than after it.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the ban's own note directs a driver that reads the clock to expect-attribute it,                   as crate::blocking does; this test measures elapsed time because that is the                   property under test -- that the bound applies before the walk rather than after"
    )]
    fn a_sheet_naming_a_row_two_billion_down_is_refused_in_milliseconds() {
        let Ok(bytes) = crafted_far_row_workbook() else {
            panic!("the crafted fixture writes");
        };
        assert!(
            bytes.len() < 8_192,
            "the whole attack is a few kilobytes, which is what makes it one: {} bytes",
            bytes.len()
        );

        let started = std::time::Instant::now();
        let refused = grids("crafted.xlsx", &bytes);
        let elapsed = started.elapsed();

        // The variant is the deterministic half of the proof and the one that
        // cannot flake: `RowOutOfRange` is constructible only inside the
        // streaming loop, which runs before `grid_of` exists to walk anything.
        // Seeing it therefore establishes the ordering by construction.
        assert!(
            matches!(refused, Err(Malformed::RowOutOfRange { .. })),
            "a row past the ceiling is refused by name: {refused:?}"
        );
        // The clock is the second half, and it guards a different thing: a
        // future refactor that moved the check after the walk would still
        // produce the variant above and would take minutes here. Generous
        // against a loaded test machine and four orders under the walk it
        // refuses.
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "the refusal must land before the range is walked, and took {elapsed:?}"
        );
    }

    /// A workbook with a header row and one cell two billion rows down, which
    /// is under every other bound this module applies.
    fn crafted_far_row_workbook() -> Result<Vec<u8>, rust_xlsxwriter::XlsxError> {
        use rust_xlsxwriter::{Workbook, Worksheet};
        let mut book = Workbook::new();
        let mut sheet = Worksheet::new();
        sheet.set_name("TES")?;
        // A header, so the emptiness check upstream of the walk passes and the
        // test exercises the bound rather than an earlier refusal.
        sheet.write_string(0, 0, "Status")?;
        // Excel's own last row. calamine accumulates a reference's row into a
        // plain u32 with no limit of its own, so a real file can name further;
        // this is as far as the writer will go, and it is four orders past the
        // ceiling, which is all the test needs.
        sheet.write_string(1_048_575, 0, "x")?;
        book.push_worksheet(sheet);
        book.save_to_buffer()
    }

    #[test]
    fn a_workbook_declaring_a_huge_expansion_is_refused_before_it_is_opened() {
        // A minimal central directory naming one entry that says it unpacks to
        // a gigabyte. Nothing decompresses, which is the point: the refusal
        // reads the declaration and stops.
        let mut bytes = b"PK\x03\x04".to_vec();
        bytes.resize(64, 0);
        let entry_at = bytes.len();
        let mut entry = vec![0x50, 0x4b, 0x01, 0x02];
        entry.resize(46, 0);
        // uncompressed size at offset 24, one gigabyte
        entry[24..28].copy_from_slice(&1_073_741_824_u32.to_le_bytes());
        bytes.extend_from_slice(&entry);
        let mut end = vec![0x50, 0x4b, 0x05, 0x06];
        end.resize(22, 0);
        end[16..20].copy_from_slice(&u32::try_from(entry_at).unwrap_or(0).to_le_bytes());
        bytes.extend_from_slice(&end);

        assert!(
            matches!(
                grids("bomb.xlsx", &bytes),
                Err(Malformed::DeclaresTooLarge { .. })
            ),
            "an archive declaring a gigabyte is refused by its own declaration"
        );
    }
}
