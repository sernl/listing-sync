//! What the template workbook's columns are, derived from the registry once.
//!
//! The writer and the reader take their columns from here rather than each
//! stating them, because a template whose header the parser does not recognise
//! is a sheet the seller filled in for nothing, and two lists drift the first
//! time a native field is added. Requiredness in particular has exactly one
//! home — `tam_domain::registry` says so in its own module doc — so nothing
//! below decides it, and everything below reads it.

use tam_domain::product::TITLE_MAX_UTF16_UNITS;
use tam_domain::registry::{
    registry, Delegation, FieldDirection, LengthCap, NativeField, NativeVocabulary,
};
use tam_types::{InventoryId, LengthUnit};

/// The prose tab, which is not a grid and is never parsed.
pub const READ_ME: &str = "Read me";

/// The hidden tab the dropdowns reference by range.
///
/// Excel data validation over a long list wants a range rather than an inline
/// literal — the inline form has its own length ceiling that Tes `yearGroups`
/// alone would meet — so every captured vocabulary is written down a column
/// here and each dropdown points at its own.
pub const LISTS: &str = "Lists";

/// The status a row asks for, in the two words the column accepts.
pub const STATUS_DRAFT: &str = "draft";
pub const STATUS_LIVE: &str = "live";

/// The tab titles that are not grids, so a parse skips them by name rather
/// than by guessing at their shape.
pub const NON_GRID_TABS: [&str; 2] = [READ_ME, LISTS];

/// The denominations a price may be stated in, as `Currency::code` spells
/// them.
///
/// A grid tab carries no currency column, because the currency follows the
/// inventory and letting a seller state one the publishing path disagrees with
/// is the refusal `export.rs` already makes when it declines to compute a
/// converted price. The platform-only tab is the exception and needs one: a row
/// there names no inventory to read a currency off, `PriceIntent::Paid` carries
/// a `Money` and `Money` carries a currency, so without this column a paid
/// Teachouse row would have its denomination invented.
///
/// Held against the closed enum by [`the_currency_column_offers_every_currency`],
/// which fails to compile rather than to assert when a third is added.
pub const CURRENCIES: [&str; 2] = ["GBP", "USD"];

/// The currency a code names, or none where it names no currency this system
/// holds.
#[must_use]
pub fn currency_of(code: &str) -> Option<tam_types::Currency> {
    match code {
        "GBP" => Some(tam_types::Currency::Gbp),
        "USD" => Some(tam_types::Currency::Usd),
        _ => None,
    }
}

/// One tab of the workbook that carries rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tab {
    /// The tab's own title, which is the console's short name for the
    /// inventory so a seller reads the same word in the workbook, the console
    /// and the export.
    pub title: &'static str,
    /// The inventory a row here is authored for, or `None` on the tab whose
    /// rows live only on our platform.
    pub inventory: Option<InventoryId>,
}

impl Tab {
    /// Whether a row on this tab may say live.
    ///
    /// The Teachouse tab accepts only `draft`, because there is no marketplace
    /// for a live row here to be live on.
    #[must_use]
    pub const fn admits_live(&self) -> bool {
        self.inventory.is_some()
    }
}

/// Every grid tab, in the order the export writes its column groups.
///
/// No Etsy tab. Etsy has no adapter, so a row there could never publish, and a
/// tab that cannot work is a tab that gets filled in. The absence is held
/// against `InventoryId::ALL` by `every_inventory_is_a_tab_or_is_refused_by_name`
/// below, so an inventory added to the model is a failing test rather than a
/// tab that quietly stops existing.
pub const TABS: [Tab; 3] = [
    Tab {
        title: "Teachouse",
        inventory: None,
    },
    Tab {
        title: "TES",
        inventory: Some(InventoryId::Tes),
    },
    Tab {
        title: "TPT",
        inventory: Some(InventoryId::Tpt),
    },
];

/// The tab a title names, or none where the workbook has no such grid.
#[must_use]
pub fn tab_of(title: &str) -> Option<Tab> {
    TABS.iter().copied().find(|tab| tab.title == title)
}

/// Which cell of the create form one column fills.
///
/// A closed sum rather than a string, so a column added to the writer without
/// a reader is a compile error at the parse's own match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Status,
    ResourceId,
    Title,
    Description,
    Price,
    Labels,
    File,
    /// The denomination a paid price is stated in. Present on the platform-only
    /// tab alone; see [`CURRENCIES`].
    Currency,
    /// One of the tab inventory's own native fields, named as its wire names
    /// it.
    Native(&'static str),
}

/// The marker row 2 carries under each column name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Required,
    Optional,
    /// A field whose `Delegation` is `Never(NonDelegable::LegalContent)`: never
    /// filled by a default, a template or a computation. Today that is the Tes
    /// `licence` and TPT's `copyright_declaration`.
    ///
    /// Distinct from `Required` even where the field is also required, because
    /// the two say different things to a seller: one is "we will refuse without
    /// it", the other is "no one but you may write it".
    SellerOnly,
}

impl Marker {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Optional => "optional",
            Self::SellerOnly => "seller only",
        }
    }
}

/// What a column's cell may hold, as the writer renders it and the reader
/// checks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Values {
    /// A captured closed set. The writer backs a dropdown with it and the
    /// reader refuses anything outside it.
    Closed(&'static [&'static str]),
    /// Documented closed upstream with the members not captured. A plain cell
    /// and a read-me line: we hold no set to check against, and inventing one
    /// would refuse a value the platform accepts.
    ClosedUncaptured,
    /// Free text, bounded by `cap` where one is recorded.
    Text { cap: Option<LengthCap> },
    /// A number, with no enumerable vocabulary.
    Numeric,
}

/// One column of one grid tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub cell: Cell,
    /// The header cell, in seller words where a capture recorded them.
    pub title: String,
    /// The wire name behind the title, where the two differ, so the read-me
    /// can state it. `None` on a shared column, which has no wire name.
    pub wire_name: Option<&'static str>,
    pub marker: Marker,
    /// Whether a blank cell is a refusal. Read off the registry for a native
    /// column, so the workbook and the create cannot disagree.
    pub required: bool,
    /// Whether this column takes a set rather than one value, semicolon
    /// separated in the export's own convention.
    pub multiple: bool,
    pub values: Values,
}

/// The columns every grid tab opens with, before its native ones.
///
/// `File` is here rather than beside the marketplace columns because a live
/// row on any tab publishes bytes, and a Teachouse row may attach them too.
/// `Resource ID` is present so an exported catalogue can be edited and
/// re-uploaded later; this build creates and does not update, so a non-blank
/// value is refused by name rather than silently ignored.
fn shared_columns(tab: Tab) -> Vec<Column> {
    let statuses: &'static [&'static str] = if tab.admits_live() {
        &[STATUS_DRAFT, STATUS_LIVE]
    } else {
        &[STATUS_DRAFT]
    };
    vec![
        Column {
            cell: Cell::Status,
            title: "Status".to_owned(),
            wire_name: None,
            marker: Marker::Optional,
            required: false,
            multiple: false,
            values: Values::Closed(statuses),
        },
        Column {
            cell: Cell::ResourceId,
            title: "Resource ID".to_owned(),
            wire_name: None,
            marker: Marker::Optional,
            required: false,
            multiple: false,
            values: Values::Text { cap: None },
        },
        Column {
            cell: Cell::Title,
            title: "Title".to_owned(),
            wire_name: None,
            marker: Marker::Required,
            required: true,
            multiple: false,
            values: Values::Text {
                cap: Some(title_cap(tab)),
            },
        },
        Column {
            cell: Cell::Description,
            title: "Description".to_owned(),
            wire_name: None,
            marker: Marker::Optional,
            required: false,
            multiple: false,
            values: Values::Text {
                cap: description_cap(tab),
            },
        },
        Column {
            cell: Cell::Price,
            title: "Price".to_owned(),
            wire_name: None,
            marker: Marker::Required,
            required: true,
            multiple: false,
            values: Values::Text { cap: None },
        },
        Column {
            cell: Cell::Labels,
            title: "Labels".to_owned(),
            wire_name: None,
            marker: Marker::Optional,
            required: false,
            multiple: true,
            values: Values::Text { cap: None },
        },
        Column {
            cell: Cell::File,
            title: "File".to_owned(),
            wire_name: None,
            // Decision D32: a row naming a marketplace publishes a file there
            // whether it lists as a draft or live, so the column is required on
            // every marketplace tab and optional on the platform-only one. The
            // marker states the rule the parse enforces, rather than letting a
            // seller find it out from the report.
            marker: if tab.admits_live() {
                Marker::Required
            } else {
                Marker::Optional
            },
            required: tab.admits_live(),
            multiple: false,
            values: Values::Text { cap: None },
        },
    ]
}

/// The columns one tab adds to the shared set for reasons of its own.
///
/// One entry today: the platform-only tab's currency, which every grid tab
/// reads off its inventory instead.
fn tab_columns(tab: Tab) -> Vec<Column> {
    if tab.inventory.is_some() {
        return Vec::new();
    }
    vec![Column {
        cell: Cell::Currency,
        title: "Currency".to_owned(),
        wire_name: None,
        // Blank is the ordinary state of a free row, so the marker says
        // optional and the parse refuses a paid price that states none.
        marker: Marker::Optional,
        required: false,
        multiple: false,
        values: Values::Closed(&CURRENCIES),
    }]
}

/// The title cap this tab's rows are held to: the tighter of the inventory's
/// own recorded cap and `ProductName`'s eighty UTF-16 code units.
///
/// The eighty is `tam_domain::product::TITLE_MAX_UTF16_UNITS`, measured rather
/// than chosen: TPT's `#ItemName` carries a real `maxlength="80"`, so a browser
/// truncates there, and the constant's own comment claims nothing about what
/// TPT's server would do. It is the platform's own product-name type, which the
/// authoring form validates through and which the Template Manager applies to a
/// saved draft, so a title this import accepts is one that form accepts.
///
/// What it is *not* is a rule `POST /{version}/products` applies to every
/// create. That route's only unconditional title check is that it is not blank;
/// the length is reached through `verdict` only when the body carries a TPT-base
/// block, which this import does not send. So the import is stricter than the
/// raw API here, in the safe direction — it refuses nothing the create would
/// have kept out, and it refuses a title between eighty-one units and whatever
/// the seller typed that the create route would have stored. Whether the create
/// route should enforce its own domain type is a question about `catalogue.rs`
/// rather than about this module, and it is recorded rather than answered here.
///
/// A tighter platform cap from the registry binds on top, and a platform with no
/// recorded cap contributes nothing rather than a guess.
fn title_cap(tab: Tab) -> LengthCap {
    let domain = LengthCap {
        limit: TITLE_MAX_UTF16_UNITS,
        unit: LengthUnit::Utf16CodeUnits,
    };
    let Some(inventory) = tab.inventory else {
        return domain;
    };
    match registry(inventory).canonical.title.cap {
        // Two caps in different units cannot be compared, so the tighter one
        // is not decidable and both are enforced instead: the parser checks
        // each in its own unit. The writer states the domain type's, which is
        // the one a seller can count.
        Some(platform) if platform.unit == domain.unit => LengthCap {
            limit: platform.limit.min(domain.limit),
            unit: domain.unit,
        },
        Some(_) | None => domain,
    }
}

/// The description cap the tab's inventory records, or none where it records
/// nothing. Absent means unmeasured, never unlimited.
fn description_cap(tab: Tab) -> Option<LengthCap> {
    tab.inventory
        .and_then(|inventory| registry(inventory).canonical.description.cap)
}

/// Whether one native field earns a column.
///
/// A `ReadOnly` native is omitted because it is something we learn from the
/// platform rather than something a seller states. An `Unmeasured` vocabulary
/// is omitted because a cell nobody can fill correctly is worse than an absent
/// one: we hold neither a set to offer nor evidence that free text is accepted.
fn writable(native: &NativeField) -> bool {
    matches!(
        native.direction,
        FieldDirection::Written | FieldDirection::Both
    ) && !matches!(native.vocabulary, NativeVocabulary::Unmeasured)
}

fn native_column(inventory: InventoryId, native: &NativeField) -> Column {
    let marker = if matches!(native.delegation, Delegation::Never(_)) {
        Marker::SellerOnly
    } else if native.required {
        Marker::Required
    } else {
        Marker::Optional
    };
    let values = match native.vocabulary {
        NativeVocabulary::Closed(members) => Values::Closed(members),
        NativeVocabulary::ClosedUncaptured => Values::ClosedUncaptured,
        NativeVocabulary::Numeric => Values::Numeric,
        // Free text and an unmeasured vocabulary render the same cell and mean
        // different things: one is a platform that accepts prose, the other is
        // a field we have established nothing about. `writable` above refuses
        // the second a column at all, so that arm is unreachable, and both are
        // spelled out rather than left to a wildcard because that is what makes
        // a sixth vocabulary a compile error here.
        NativeVocabulary::Free | NativeVocabulary::Unmeasured => Values::Text { cap: None },
    };
    Column {
        cell: Cell::Native(native.name),
        // The wire name where no capture recorded a reading. `None` is not a
        // hole to fill: inventing a label would put a claim on a seller's
        // screen no capture supports.
        title: native.label.unwrap_or(native.name).to_owned(),
        wire_name: Some(native.name),
        marker,
        required: native.required,
        multiple: matches!(
            registry(inventory)
                .equivalence_axes
                .iter()
                .find(|binding| binding.native == native.name)
                .map(|binding| binding.cardinality),
            Some(tam_domain::registry::Cardinality::Many { .. })
        ),
        values,
    }
}

/// Every column of one grid tab, shared ones first and the inventory's own
/// native ones after, in the registry's own order.
#[must_use]
pub fn columns(tab: Tab) -> Vec<Column> {
    let mut columns = shared_columns(tab);
    columns.extend(tab_columns(tab));
    if let Some(inventory) = tab.inventory {
        columns.extend(
            registry(inventory)
                .natives
                .iter()
                .filter(|native| writable(native))
                .map(|native| native_column(inventory, native)),
        );
    }
    columns
}

/// The one worked example row the template writes under each header, and the
/// row a parse skips when it finds it unchanged.
///
/// One function rather than a literal in the writer and a rule in the reader,
/// because the two have to agree exactly: the reader skips this row only where
/// every cell of it matches, and treats an edited one as the seller's own data.
/// That is what stops a seller who typed over the example from losing a row
/// silently, which is the failure a position-based skip has and this does not.
///
/// A `ClosedUncaptured` or free-text native gets a blank example rather than an
/// invented value: we hold no member of that set, and putting a plausible one
/// in front of a seller is exactly the guess the registry's three-way
/// vocabulary distinction exists to prevent.
#[must_use]
pub fn example_cell(column: &Column) -> String {
    match column.cell {
        Cell::Status => STATUS_DRAFT.to_owned(),
        // Blank on a new row, which is what an example row is.
        Cell::ResourceId => String::new(),
        Cell::Title => "Fractions of amounts: 24 task cards".to_owned(),
        Cell::Description => {
            "Twenty-four task cards for finding fractions of amounts, with an answer key."
                .to_owned()
        }
        Cell::Price => "3.50".to_owned(),
        Cell::Labels => "Autumn Term; Year 5".to_owned(),
        Cell::File => "fractions-task-cards.pdf".to_owned(),
        Cell::Currency => "GBP".to_owned(),
        Cell::Native(_) => match column.values {
            Values::Closed(members) => members
                .first()
                .map(|first| (*first).to_owned())
                .unwrap_or_default(),
            Values::ClosedUncaptured | Values::Text { .. } | Values::Numeric => String::new(),
        },
    }
}

/// The whole example row of one tab, in column order.
#[must_use]
pub fn example_row(tab: Tab) -> Vec<String> {
    columns(tab).iter().map(example_cell).collect()
}

/// Every captured vocabulary the workbook's dropdowns reference, each under the
/// column title that uses it, in the order the tabs and columns are written.
///
/// Deduplicated on the members rather than on the title, because the three Tes
/// tabs share a licence set and writing it three times would make the hidden
/// tab three times as wide for no reader.
#[must_use]
pub fn vocabularies() -> Vec<(String, &'static [&'static str])> {
    let mut out: Vec<(String, &'static [&'static str])> = Vec::new();
    for tab in TABS {
        for column in columns(tab) {
            let Values::Closed(members) = column.values else {
                continue;
            };
            if out
                .iter()
                .any(|(_, held)| std::ptr::eq(*held, members) || *held == members)
            {
                continue;
            }
            out.push((column.title.clone(), members));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        columns, tab_of, vocabularies, Cell, Marker, Values, LISTS, NON_GRID_TABS, READ_ME,
        STATUS_DRAFT, STATUS_LIVE, TABS,
    };
    use tam_domain::registry::{registry, Delegation, FieldDirection, NativeVocabulary};
    use tam_types::InventoryId;

    /// The forcing function the export's own column test gives it: an
    /// inventory added to the model either gets a tab or is refused here by
    /// name, so it cannot quietly stop being importable.
    #[test]
    fn every_inventory_is_a_tab_or_is_refused_by_name() {
        for inventory in InventoryId::ALL {
            let has_tab = TABS.iter().any(|tab| tab.inventory == Some(inventory));
            let expected = match inventory {
                InventoryId::Tes | InventoryId::Tpt => true,
                // No adapter, so a row on an Etsy tab could never publish.
                InventoryId::Etsy => false,
            };
            assert_eq!(
                has_tab, expected,
                "{inventory:?} must be a tab exactly when this build can publish to it"
            );
        }
    }

    #[test]
    fn the_platform_only_tab_admits_no_live_row() {
        let Some(tab) = tab_of("Teachouse") else {
            panic!("the platform-only tab is in TABS");
        };
        assert!(
            !tab.admits_live(),
            "a Teachouse row names no marketplace, so there is nothing for it to be live on"
        );
        let statuses = columns(tab)
            .into_iter()
            .find(|column| column.cell == Cell::Status)
            .map(|column| column.values);
        assert_eq!(
            statuses,
            Some(Values::Closed(&[STATUS_DRAFT])),
            "the status dropdown offers only what the tab admits"
        );
    }

    #[test]
    fn every_grid_tab_admitting_live_offers_both_statuses() {
        for tab in TABS.into_iter().filter(super::Tab::admits_live) {
            let statuses = columns(tab)
                .into_iter()
                .find(|column| column.cell == Cell::Status)
                .map(|column| column.values);
            assert_eq!(
                statuses,
                Some(Values::Closed(&[STATUS_DRAFT, STATUS_LIVE])),
                "{} is a marketplace tab, so a row there may say live",
                tab.title
            );
        }
    }

    /// The registry is the one home for requiredness, so no column may state a
    /// requirement its own native field does not.
    #[test]
    fn a_native_columns_requiredness_is_the_registrys_own() {
        for tab in TABS {
            let Some(inventory) = tab.inventory else {
                continue;
            };
            for column in columns(tab) {
                let Cell::Native(name) = column.cell else {
                    continue;
                };
                let Some(native) = registry(inventory).native(name) else {
                    panic!("{name} is a column of {} and must be a native", tab.title);
                };
                assert_eq!(
                    column.required, native.required,
                    "{}'s {name} column must state the registry's own requiredness",
                    tab.title
                );
            }
        }
    }

    #[test]
    fn a_never_delegable_native_is_marked_seller_only_even_when_required() {
        let mut marked = 0_usize;
        for tab in TABS {
            let Some(inventory) = tab.inventory else {
                continue;
            };
            for column in columns(tab) {
                let Cell::Native(name) = column.cell else {
                    continue;
                };
                let Some(native) = registry(inventory).native(name) else {
                    panic!("{name} is a column of {} and must be a native", tab.title);
                };
                if matches!(native.delegation, Delegation::Never(_)) {
                    assert_eq!(
                        column.marker,
                        Marker::SellerOnly,
                        "{}'s {name} refuses delegation, so the workbook says so rather than \
                         calling it required",
                        tab.title
                    );
                    marked = marked.saturating_add(1);
                }
            }
        }
        assert_eq!(marked, 2, "the Tes licence and TPT's copyright declaration");
    }

    #[test]
    fn a_read_only_or_unmeasured_native_gets_no_column() {
        for tab in TABS {
            let Some(inventory) = tab.inventory else {
                continue;
            };
            let held: Vec<&str> = columns(tab)
                .into_iter()
                .filter_map(|column| match column.cell {
                    Cell::Native(name) => Some(name),
                    Cell::Status
                    | Cell::ResourceId
                    | Cell::Title
                    | Cell::Description
                    | Cell::Price
                    | Cell::Currency
                    | Cell::Labels
                    | Cell::File => None,
                })
                .collect();
            for native in registry(inventory).natives {
                let expected = matches!(
                    native.direction,
                    FieldDirection::Written | FieldDirection::Both
                ) && !matches!(native.vocabulary, NativeVocabulary::Unmeasured);
                assert_eq!(
                    held.contains(&native.name),
                    expected,
                    "{}'s {} must have a column exactly when a seller can fill one",
                    tab.title,
                    native.name
                );
            }
        }
    }

    /// The currency column's members and the closed enum are the same set.
    ///
    /// The match below is what makes this a forcing function rather than an
    /// assertion: `wildcard_enum_match_arm` is denied workspace-wide, so a
    /// third currency fails to compile here and the workbook cannot silently
    /// stop offering one.
    #[test]
    fn the_currency_column_offers_every_currency() {
        use tam_types::Currency;
        const fn code(currency: Currency) -> &'static str {
            match currency {
                Currency::Gbp => "GBP",
                Currency::Usd => "USD",
            }
        }
        for currency in [Currency::Gbp, Currency::Usd] {
            assert_eq!(
                code(currency),
                currency.code(),
                "the column's spelling is the type's own"
            );
            assert!(
                super::CURRENCIES.contains(&currency.code()),
                "{} is a currency a price can be stated in, so the column offers it",
                currency.code()
            );
            assert_eq!(
                super::currency_of(currency.code()),
                Some(currency),
                "a code the column offers reads back as the currency it names"
            );
        }
    }

    /// A grid tab reads its denomination off its inventory, so offering a
    /// column would let a seller state one the publishing path disagrees with.
    #[test]
    fn only_the_platform_only_tab_carries_a_currency_column() {
        for tab in TABS {
            let carries = columns(tab)
                .iter()
                .any(|column| column.cell == Cell::Currency);
            assert_eq!(
                carries,
                tab.inventory.is_none(),
                "{} carries a currency column exactly when it has no inventory to read one from",
                tab.title
            );
        }
    }

    #[test]
    fn no_grid_tab_shares_a_title_with_the_prose_or_list_tabs() {
        for tab in TABS {
            assert!(
                !NON_GRID_TABS.contains(&tab.title),
                "{} would be parsed as a grid and written as prose",
                tab.title
            );
        }
        assert_ne!(READ_ME, LISTS, "the two non-grid tabs are two tabs");
    }

    #[test]
    fn no_tab_repeats_a_column_title() {
        for tab in TABS {
            let mut titles: Vec<String> = columns(tab)
                .into_iter()
                .map(|column| column.title)
                .collect();
            let held = titles.len();
            titles.sort();
            titles.dedup();
            assert_eq!(
                titles.len(),
                held,
                "{}'s header must address each column once, or a parse cannot tell two apart",
                tab.title
            );
        }
    }

    #[test]
    fn every_captured_vocabulary_a_column_offers_is_written_to_the_lists_tab() {
        let listed = vocabularies();
        for tab in TABS {
            for column in columns(tab) {
                let Values::Closed(members) = column.values else {
                    continue;
                };
                assert!(
                    listed.iter().any(|(_, held)| *held == members),
                    "{}'s {} offers a set no range on the hidden tab holds",
                    tab.title,
                    column.title
                );
            }
        }
    }
}
