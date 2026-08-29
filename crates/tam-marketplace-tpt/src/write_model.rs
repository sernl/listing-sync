//! The write model: the listing shape TPT's two product forms accept, the
//! small integer vocabularies they encode it in, and the two ordered field
//! builders that render it.
//!
//! The field builders emit names and values in the wire order the captures
//! record. Order is not decoration here: CakePHP's `SecurityComponent` hashes
//! a field-name set, the multipart body is the only place several of these
//! names appear, and reproducing the recorded sequence is the cheapest way to
//! stay inside a shape a live server has already accepted.
//!
//! The `FieldSet` contract is this crate's on both sides, as it is on Tes:
//! [`project_fields`] renders it from a projected listing and
//! [`listing_from_field_set`] parses it back, so both halves sit in one file.
//! `Title` is plain text, `Description` is HTML, `Price` is JSON
//! `{"free": bool, ...}`, `Taxonomy` is JSON `{"tags": [..], "categories": [..]}`
//! and `Grades` is JSON `{"tags": [..]}`.

use pulldown_cmark::{html::push_html, Options, Parser};
use serde_json::{json, Value};
use tam_marketplace::{AdapterError, FieldSet, ProjectedListing};
use tam_types::{
    CopyFormat, CurrencyRule, FailureCode, FailureDetail, FieldKey, InventoryId, PriceIntent,
    Timestamp,
};

use crate::form::{ThumbHandle, TptFormTokens};
use crate::upload::ProcessedHandle;

/// The seller's declaration that the resource is their own original work,
/// which `data[ItemsProperty][copyright_declaration]` carries as `1`.
///
/// It is a legal attestation, so it is state and not a constant: the only way
/// to obtain one is [`AuthorshipDeclaration::attested`], which names the act
/// and records who performed it and when. Nothing in this crate can put a `1`
/// on the wire without such a value existing, which is the whole point of the
/// type — a hard-coded literal would have this connector attesting on a
/// seller's behalf to something no one asked them.
///
/// M7 carries the attestation on the adapter, set when the connection is
/// linked. Persisting it per connection is a follow-up; no column exists yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorshipDeclaration {
    attested_by: String,
    attested_at: Timestamp,
}

impl AuthorshipDeclaration {
    /// The seller attested, naming themselves and the instant. There is no
    /// `Default`, no `from_bool` and no other constructor.
    #[must_use]
    pub const fn attested(attested_by: String, attested_at: Timestamp) -> Self {
        Self {
            attested_by,
            attested_at,
        }
    }

    #[must_use]
    pub fn attested_by(&self) -> &str {
        &self.attested_by
    }

    #[must_use]
    pub const fn attested_at(&self) -> Timestamp {
        self.attested_at
    }

    /// The wire value. `UploadPageProductQuery` reads the same field back as
    /// `ORIGINAL_WORK`, which is the only member either capture shows.
    #[must_use]
    pub const fn wire_value(&self) -> &'static str {
        "1"
    }
}

/// Whether a submit leaves the product a draft or makes it live. Both members
/// are captured — `0` on the create, `1` on the edit that published — and the
/// read side names the second `ACTIVE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusUser {
    Draft,
    Live,
}

impl StatusUser {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "0",
            Self::Live => "1",
        }
    }
}

/// Two fractional digits, which is the scale both product forms render money
/// at: the captured paid edit posted `3.00` against a `2.70` licence price.
const MINOR_UNITS_PER_UNIT: i64 = 100;

/// The form's own floor, from `min_price: 0.95` in the page bootstrap of both
/// captured renders. It is enforced here because a submit the form refuses is
/// answered by a re-rendered page, which [`crate::classify::classify_submit`]
/// must read as an ambiguity rather than as a rejection: refusing a
/// below-minimum price locally is what keeps it from becoming a write nobody
/// can reconcile.
pub const MIN_PRICE_MINOR_UNITS: i64 = 95;

/// `multiple_license_price_percentage: 90` from the same bootstrap, and the
/// captured paid edit is the arithmetic: `price` 3.00 against `license_price`
/// 2.70.
pub const LICENSE_PRICE_PERCENT: i64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceError {
    BelowMinimum { minor_units: i64 },
    NotRepresentable { minor_units: i64 },
}

impl core::fmt::Display for PriceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::BelowMinimum { minor_units } => write!(
                f,
                "a price of {minor_units} minor units is under the {MIN_PRICE_MINOR_UNITS} the \
                 product form states as its own minimum"
            ),
            Self::NotRepresentable { minor_units } => write!(
                f,
                "a price of {minor_units} minor units does not render as an amount the product \
                 form accepts"
            ),
        }
    }
}

impl core::error::Error for PriceError {}

/// Rounds half up, which is a choice rather than a measurement: the one
/// captured pair is 3.00 against 2.70, where ninety percent is a whole
/// hundredth and every rounding rule agrees. A price whose ninetieth part
/// falls between two hundredths — 1.99 does — is settled here so that it is
/// settled somewhere legible.
fn licence_minor_units(minor_units: i64) -> Option<i64> {
    minor_units
        .checked_mul(LICENSE_PRICE_PERCENT)?
        .checked_add(MINOR_UNITS_PER_UNIT.checked_div(2)?)?
        .checked_div(MINOR_UNITS_PER_UNIT)
}

fn decimal(minor_units: i64) -> Option<String> {
    let units = minor_units.checked_div(MINOR_UNITS_PER_UNIT)?;
    let hundredths = minor_units.checked_rem(MINOR_UNITS_PER_UNIT)?;
    Some(format!("{units}.{hundredths:02}"))
}

/// A price the product forms will carry, rendered once at construction so no
/// field builder has arithmetic left to fail at.
///
/// TPT writes money as a bare decimal — `data[Item][price]` is `3.00` and no
/// field on either form names a currency — so this carries an amount and
/// states no denomination. TPT's own currency rule is `SellerScoped` and
/// unverified, which makes what the founder's store is denominated in a
/// founder question and not one this connector may answer by picking a
/// symbol; the projection keeps its `Currency` where it can still be compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaidPrice {
    minor_units: i64,
    amount: String,
    licence_amount: String,
}

impl PaidPrice {
    /// The amount the projection deliberately set, in its own minor units.
    /// There is no other constructor: a price reaches the wire only by having
    /// been named, and never by defaulting.
    pub fn new(minor_units: i64) -> Result<Self, PriceError> {
        if minor_units < MIN_PRICE_MINOR_UNITS {
            return Err(PriceError::BelowMinimum { minor_units });
        }
        let licence =
            licence_minor_units(minor_units).ok_or(PriceError::NotRepresentable { minor_units })?;
        Ok(Self {
            minor_units,
            amount: decimal(minor_units).ok_or(PriceError::NotRepresentable { minor_units })?,
            licence_amount: decimal(licence).ok_or(PriceError::NotRepresentable { minor_units })?,
        })
    }

    #[must_use]
    pub const fn minor_units(&self) -> i64 {
        self.minor_units
    }

    /// `data[Item][price]`.
    #[must_use]
    pub fn amount(&self) -> &str {
        &self.amount
    }

    /// `data[Item][license_price]`, the ninety percent the form derives.
    #[must_use]
    pub fn licence_amount(&self) -> &str {
        &self.licence_amount
    }
}

/// What a submit posts for money. The free shape is the captured create's —
/// `free` 1 against three zeroed amounts — and the paid shape is the captured
/// edit's, `price` and `license_price` carrying amounts.
///
/// The paid create itself is inference, and the only one in this file: no
/// paid create exists on the wire, because both captured creates are free.
/// `free` 0 beside a price is therefore the free create's flag inverted and
/// the paid *edit's* amounts moved onto the create form that declares the
/// same field names — sound, and still an inference until a live paid create
/// settles it. It is reachable only when a projection carries
/// `PriceIntent::Paid`, which is a price somebody set on purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingPrice {
    Free,
    Paid(PaidPrice),
}

impl ListingPrice {
    /// `data[Item][free]`, as the create form posts it: always present,
    /// carrying the flag either way.
    #[must_use]
    pub const fn free_flag(&self) -> &'static str {
        match *self {
            Self::Free => "1",
            Self::Paid(_) => "0",
        }
    }

    /// `data[Item][free]`, as the edit form posts it: present on a free
    /// listing and absent on a priced one.
    ///
    /// The captured edit is of a paid product and posts no `free` at all,
    /// which this file once read as the edit form never carrying the field.
    /// Measured live on 2026-08-29 against a free draft, that omission is what
    /// a paid edit looks like and not what the form permits generally: the
    /// edit body without `free` was answered 302 back to the edit route with
    /// the listing untouched, and the same body carrying `free` 1 answered 302
    /// to `/Product/…` and applied on the next read. Three other differences
    /// from the capture — an empty category, a page count and a tax code —
    /// were each posted alone and each still bounced, so this flag is the one
    /// the form was missing.
    #[must_use]
    pub const fn free_flag_on_edit(&self) -> Option<&'static str> {
        match *self {
            Self::Free => Some(self.free_flag()),
            Self::Paid(_) => None,
        }
    }

    #[must_use]
    pub fn amount(&self) -> &str {
        match *self {
            Self::Free => "0",
            Self::Paid(ref paid) => paid.amount(),
        }
    }

    #[must_use]
    pub fn licence_amount(&self) -> &str {
        match *self {
            Self::Free => "0",
            Self::Paid(ref paid) => paid.licence_amount(),
        }
    }
}

/// TPT's five tax codes, the one vocabulary here that is captured whole:
/// `TaxCodesQuery` returned all five rows, and the create form posts the row
/// id rather than the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaxCode {
    DigitalAudio,
    DigitalBooks,
    DigitalImages,
    Videos,
    OtherDigitalGoods,
}

impl TaxCode {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::DigitalAudio => "1",
            Self::DigitalBooks => "2",
            Self::DigitalImages => "3",
            Self::Videos => "4",
            Self::OtherDigitalGoods => "5",
        }
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::DigitalAudio => "DA051011",
            Self::DigitalBooks => "DB031013",
            Self::DigitalImages => "DI010200",
            Self::Videos => "DV010200",
            Self::OtherDigitalGoods => "DO010000",
        }
    }
}

/// A small integer whose vocabulary is not captured, carried as the digits
/// the wire carried. One observed member is not a vocabulary, so these are
/// values with provenance rather than enums with invented names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireCode(&'static str);

impl WireCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// `data[Item][generate_thumbnail]`. The create posted `1` and the edit `3`,
/// and no source states what either means, so each submit reproduces the
/// value its own capture carried and neither is generalised.
pub const GENERATE_THUMBNAIL_ON_CREATE: WireCode = WireCode("1");
/// See [`GENERATE_THUMBNAIL_ON_CREATE`].
pub const GENERATE_THUMBNAIL_ON_EDIT: WireCode = WireCode("3");

/// `data[ItemsProperty][duration]`. `0` on the create, `6` on the edit where
/// the read side named it `HOURS_1`; the rest of the scale is uncaptured.
pub const DURATION_UNSET: WireCode = WireCode("0");

/// `data[ItemsProperty][answer_key]`. `0` on the create, `1` on the edit
/// where the read side named it `INCLUDED`.
pub const ANSWER_KEY_ABSENT: WireCode = WireCode("0");

/// `data[ItemsLocalization][country_id_flag]`. `0` on the create and `1` on
/// the edit; the country id itself is never posted by either.
pub const COUNTRY_FLAG_OFF: WireCode = WireCode("0");

/// `thumbs`. Sent verbatim as the create posted it. Whether it counts manual
/// thumbnails, selects between generated and manual, or means something else
/// is unsettled, so its captured value is reproduced rather than derived.
pub const THUMBS_VERBATIM: &str = "0";

/// The listing as TPT's product forms accept it. Everything platform-neutral
/// has already been resolved by [`project_fields`]; what remains is TPT's own
/// vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TptListing {
    pub title: String,
    /// HTML. The description editor posts markup, and the captured create
    /// carried `<p>` tags and an editor comment, so the canonical body is
    /// posted verbatim.
    ///
    /// A body canonicalised from a marketplace whose editor is markdown — the
    /// sibling Tes adapter's is — reaches this field already rendered, by
    /// `project_fields` and on the declared format rather than on a reading
    /// of the bytes. Guessing a body's format from its bytes is how a listing
    /// acquires escaped markup nobody asked for, so the declaration travels
    /// and nothing here sniffs.
    pub description_html: String,
    /// The flat slug namespace: grade, subject, audience, resource type and
    /// file format, undifferentiated, exactly as the read side returns them.
    pub taxonomy_tags: Vec<String>,
    /// Seller-owned shelves, addressed by their numeric ids.
    pub category_ids: Vec<String>,
    /// What the three money fields carry. Free on both captured creates;
    /// priced on the captured edit.
    pub price: ListingPrice,
    /// Absent on the captured create and present on the captured edit. A
    /// projection that names none posts none.
    pub tax_code: Option<TaxCode>,
}

/// The wire field names, in one place, because several of them are the only
/// evidence of what the form expects and a typo in one is a blackholed write.
mod names {
    pub(super) const METHOD: &str = "_method";
    pub(super) const TOKEN_KEY: &str = "data[_Token][key]";
    pub(super) const CSRF_KEY: &str = "data[_Csrf][csrfKey]";
    pub(super) const CSRF_TOKEN: &str = "data[_Csrf][csrfToken]";
    pub(super) const NAME: &str = "data[Item][name]";
    pub(super) const PRODUCT: &str = "data[ItemDigital][product]";
    pub(super) const PRODUCT_UPLOADED: &str = "data[ItemsProperty][product_uploaded]";
    pub(super) const PREVIEW: &str = "data[ItemDigital][preview]";
    pub(super) const PREVIEW_UPLOADED: &str = "data[ItemsProperty][preview_uploaded]";
    pub(super) const VIDEOPREVIEW: &str = "data[Upload][videopreview]";
    pub(super) const CUSTOM_VIDEOPREVIEW_UPLOADED: &str =
        "data[Upload][custom_videopreview_uploaded]";
    pub(super) const GENERATE_THUMBNAIL: &str = "data[Item][generate_thumbnail]";
    pub(super) const THUMBS: &str = "thumbs";
    pub(super) const THUMBS_COLLECTION_KEY: &str = "thumbs_collection_key";
    pub(super) const COMMON_CORE_NUM: &str =
        "data[ItemsCommonCoreStandard][common_core_standards_num]";
    pub(super) const PAGES: &str = "data[ItemsProperty][pages]";
    pub(super) const COPYRIGHT_DECLARATION: &str = "data[ItemsProperty][copyright_declaration]";
    pub(super) const TAXONOMY_TAGS: &str = "data[TaxonomyTags][]";
    pub(super) const TOKEN_FIELDS: &str = "data[_Token][fields]";
    pub(super) const TOKEN_UNLOCKED: &str = "data[_Token][unlocked]";
    pub(super) const DESCRIPTION: &str = "data[Item][description]";
    pub(super) const FREE: &str = "data[Item][free]";
    pub(super) const PRICE: &str = "data[Item][price]";
    pub(super) const DISCOUNTPRICE: &str = "data[Item][discountprice]";
    pub(super) const LICENSE_PRICE: &str = "data[Item][license_price]";
    pub(super) const STATUS_USER: &str = "data[Item][status_user]";
    pub(super) const CATEGORY: &str = "data[Category][Category][]";
    pub(super) const COMMON_CORE_ID: &str =
        "data[ItemsCommonCoreStandard][common_core_standard_id][]";
    pub(super) const COUNTRY_ID_FLAG: &str = "data[ItemsLocalization][country_id_flag]";
    pub(super) const DURATION: &str = "data[ItemsProperty][duration]";
    pub(super) const ANSWER_KEY: &str = "data[ItemsProperty][answer_key]";
    pub(super) const TAX_CODE_ID: &str = "data[ItemTaxCode][tax_code_id]";
    pub(super) const IS_POST: &str = "data[RevisedItem][is_post]";

    /// The four manual-thumbnail slots, as `(handle, uploaded-flag)` pairs.
    pub(super) const THUMB_SLOTS: [(&str, &str); 4] = [
        (
            "data[ItemDigital][thumb1]",
            "data[ItemsProperty][thumb1_uploaded]",
        ),
        (
            "data[ItemDigital][thumb2]",
            "data[ItemsProperty][thumb2_uploaded]",
        ),
        (
            "data[ItemDigital][thumb3]",
            "data[ItemsProperty][thumb3_uploaded]",
        ),
        (
            "data[ItemDigital][thumb4]",
            "data[ItemsProperty][thumb4_uploaded]",
        ),
    ];
}

fn field(name: &str, value: &str) -> (String, String) {
    (name.to_owned(), value.to_owned())
}

/// Everything the create submit needs beyond the listing itself. A struct
/// rather than five arguments, which is also what the argument-count lint
/// asks for.
#[derive(Debug, Clone, Copy)]
pub struct CreateSubmission<'a> {
    pub tokens: &'a TptFormTokens,
    pub listing: &'a TptListing,
    pub product: &'a ProcessedHandle,
    /// The 88-character handle the thumbnail job's terminal poll returned.
    pub thumbs_collection_key: &'a str,
    pub authorship: &'a AuthorshipDeclaration,
}

/// The create body: the 43 fields the captured `POST /My-Products/New/Digital-Next`
/// carried, in its order, with the two opaque handles in place.
///
/// The product is created as a draft. `DraftThenPublish` is TPT's safe create
/// strategy precisely because `status_user` is a field rather than a route: a
/// re-run that lands twice leaves two drafts, not two live products.
#[must_use]
pub fn create_fields(submission: &CreateSubmission<'_>) -> Vec<(String, String)> {
    let CreateSubmission {
        tokens,
        listing,
        product,
        thumbs_collection_key,
        authorship,
    } = *submission;
    let mut fields = vec![
        field(names::METHOD, "POST"),
        field(names::TOKEN_KEY, tokens.token_key()),
        field(names::CSRF_KEY, tokens.csrf_key()),
        field(names::CSRF_TOKEN, tokens.csrf_token()),
        field(names::NAME, &listing.title),
        field(names::PRODUCT, product.as_str()),
        field(names::PRODUCT_UPLOADED, "1"),
        field(names::PREVIEW_UPLOADED, "0"),
        field(names::CUSTOM_VIDEOPREVIEW_UPLOADED, "0"),
        field(
            names::GENERATE_THUMBNAIL,
            GENERATE_THUMBNAIL_ON_CREATE.as_str(),
        ),
        field(names::THUMBS, THUMBS_VERBATIM),
        field(names::THUMBS_COLLECTION_KEY, thumbs_collection_key),
    ];
    for (handle, uploaded) in names::THUMB_SLOTS {
        fields.push(field(handle, ""));
        fields.push(field(uploaded, "0"));
    }
    fields.push(field(names::COMMON_CORE_NUM, "0"));
    fields.push(field(names::PAGES, ""));
    fields.push(field(names::COPYRIGHT_DECLARATION, authorship.wire_value()));
    for tag in &listing.taxonomy_tags {
        fields.push(field(names::TAXONOMY_TAGS, tag));
    }
    fields.push(field(names::TOKEN_FIELDS, tokens.token_fields()));
    fields.push(field(names::TOKEN_UNLOCKED, tokens.token_unlocked()));
    fields.push(field(names::DESCRIPTION, &listing.description_html));
    fields.push(field(names::FREE, listing.price.free_flag()));
    fields.push(field(names::PRICE, listing.price.amount()));
    // Zero on every captured submit, free and paid alike: a sale price is a
    // seller action neither capture performs.
    fields.push(field(names::DISCOUNTPRICE, "0"));
    fields.push(field(names::LICENSE_PRICE, listing.price.licence_amount()));
    fields.push(field(names::STATUS_USER, StatusUser::Draft.as_str()));
    if listing.category_ids.is_empty() {
        // A create that selects no shelf still posts the name, with an empty
        // value: that is what the 2026-08-28 create capture recorded, and it
        // is the only evidence of what the form expects when the seller
        // picked nothing. The edit form has no such capture and posts what it
        // has.
        fields.push(field(names::CATEGORY, ""));
    }
    for category in &listing.category_ids {
        fields.push(field(names::CATEGORY, category));
    }
    fields.push(field(names::COMMON_CORE_ID, ""));
    fields.push(field(names::COUNTRY_ID_FLAG, COUNTRY_FLAG_OFF.as_str()));
    fields.push(field(names::DURATION, DURATION_UNSET.as_str()));
    fields.push(field(names::ANSWER_KEY, ANSWER_KEY_ABSENT.as_str()));
    fields.push(field(names::IS_POST, ""));
    fields
}

/// Everything the edit submit needs beyond the listing.
#[derive(Debug, Clone, Copy)]
pub struct EditSubmission<'a> {
    pub tokens: &'a TptFormTokens,
    pub listing: &'a TptListing,
    /// The existing thumbnail handles lifted from the same render. An edit
    /// that drops these drops the product's thumbnails.
    pub thumbs: &'a [ThumbHandle],
    pub status: StatusUser,
    pub authorship: &'a AuthorshipDeclaration,
}

/// The edit body: the fields the captured `POST /itemsDigital/editNext/{id}`
/// carried, in its order, plus the one a free listing adds.
///
/// An edit is a full replace, so every name is posted even where its value is
/// empty. Empty in `product`, `preview` and `videopreview` paired with an
/// `*_uploaded` flag of `1` is how the form says "this asset is unchanged" —
/// the captured edit re-uploaded nothing and touched no S3 host.
///
/// `free` follows [`ListingPrice::free_flag_on_edit`]: a free listing posts
/// it and a priced one does not, which is what the captured paid edit and the
/// live free edit respectively recorded.
#[must_use]
pub fn edit_fields(submission: &EditSubmission<'_>) -> Vec<(String, String)> {
    let EditSubmission {
        tokens,
        listing,
        thumbs,
        status,
        authorship,
    } = *submission;
    let mut fields = vec![
        field(names::METHOD, "POST"),
        field(names::TOKEN_KEY, tokens.token_key()),
        field(names::CSRF_KEY, tokens.csrf_key()),
        field(names::CSRF_TOKEN, tokens.csrf_token()),
        field(names::NAME, &listing.title),
        field(names::PRODUCT, ""),
        field(names::PRODUCT_UPLOADED, "1"),
        field(names::PREVIEW, ""),
        field(names::PREVIEW_UPLOADED, "0"),
        field(names::VIDEOPREVIEW, ""),
        field(names::CUSTOM_VIDEOPREVIEW_UPLOADED, "0"),
        field(names::THUMBS, ""),
        field(names::THUMBS_COLLECTION_KEY, ""),
    ];
    for (slot, (handle, uploaded)) in names::THUMB_SLOTS.into_iter().enumerate() {
        let existing = thumbs
            .iter()
            .find(|thumb| usize::from(thumb.slot()) == slot.saturating_add(1));
        if let Some(thumb) = existing {
            fields.push(field(handle, thumb.key()));
            fields.push(field(uploaded, "1"));
        } else {
            fields.push(field(handle, ""));
            fields.push(field(uploaded, "0"));
        }
    }
    fields.push(field(
        names::GENERATE_THUMBNAIL,
        GENERATE_THUMBNAIL_ON_EDIT.as_str(),
    ));
    fields.push(field(names::COMMON_CORE_NUM, "0"));
    fields.push(field(names::PAGES, ""));
    fields.push(field(names::COPYRIGHT_DECLARATION, authorship.wire_value()));
    for tag in &listing.taxonomy_tags {
        fields.push(field(names::TAXONOMY_TAGS, tag));
    }
    fields.push(field(names::TOKEN_FIELDS, tokens.token_fields()));
    fields.push(field(names::TOKEN_UNLOCKED, tokens.token_unlocked()));
    fields.push(field(names::DESCRIPTION, &listing.description_html));
    if let Some(free) = listing.price.free_flag_on_edit() {
        fields.push(field(names::FREE, free));
    }
    fields.push(field(names::PRICE, listing.price.amount()));
    fields.push(field(names::DISCOUNTPRICE, "0"));
    fields.push(field(names::LICENSE_PRICE, listing.price.licence_amount()));
    fields.push(field(names::STATUS_USER, status.as_str()));
    for category in &listing.category_ids {
        fields.push(field(names::CATEGORY, category));
    }
    fields.push(field(names::COMMON_CORE_ID, ""));
    fields.push(field(names::COUNTRY_ID_FLAG, COUNTRY_FLAG_OFF.as_str()));
    fields.push(field(names::DURATION, DURATION_UNSET.as_str()));
    fields.push(field(names::ANSWER_KEY, ANSWER_KEY_ABSENT.as_str()));
    fields.push(field(
        names::TAX_CODE_ID,
        listing.tax_code.map_or("", TaxCode::id),
    ));
    fields.push(field(names::IS_POST, ""));
    fields
}

/// The CakePHP dotted path a posted field name corresponds to, which is the
/// vocabulary `data[_Token][unlocked]` is written in. `data[Item][name]`
/// becomes `Item.name` and `data[Category][Category][]` becomes
/// `Category.Category`.
#[must_use]
pub fn dotted_path(name: &str) -> String {
    let Some(inner) = name.strip_prefix("data[") else {
        return name.trim_end_matches("[]").to_owned();
    };
    inner
        .split(['[', ']'])
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

/// Every name a create posts that is not a token or the method override:
/// `SecurityComponent` never lists its own tokens among the unlocked names,
/// so those three would read as drift on every probe.
const WRITTEN_NAMES: [&str; 22] = [
    names::NAME,
    names::PRODUCT,
    names::PRODUCT_UPLOADED,
    names::PREVIEW_UPLOADED,
    names::CUSTOM_VIDEOPREVIEW_UPLOADED,
    names::GENERATE_THUMBNAIL,
    names::THUMBS,
    names::THUMBS_COLLECTION_KEY,
    names::COMMON_CORE_NUM,
    names::PAGES,
    names::COPYRIGHT_DECLARATION,
    names::TAXONOMY_TAGS,
    names::DESCRIPTION,
    names::FREE,
    names::PRICE,
    names::DISCOUNTPRICE,
    names::LICENSE_PRICE,
    names::STATUS_USER,
    names::CATEGORY,
    names::COMMON_CORE_ID,
    names::COUNTRY_ID_FLAG,
    names::IS_POST,
];

/// The field paths a create writes, as the unlocked list names them. Sorted
/// and deduplicated, so a fingerprint over the set does not move when a
/// repeated array name is posted a different number of times.
#[must_use]
pub fn written_field_paths() -> Vec<String> {
    let mut paths: Vec<String> = WRITTEN_NAMES
        .iter()
        .copied()
        .chain(
            names::THUMB_SLOTS
                .into_iter()
                .flat_map(|(handle, uploaded)| [handle, uploaded]),
        )
        .chain([names::DURATION, names::ANSWER_KEY])
        .map(dotted_path)
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

fn refuse(detail: String) -> AdapterError {
    AdapterError::Rejected {
        code: FailureCode::UploadRejected,
        detail: FailureDetail(detail),
    }
}

/// The shape TPT issues a taxonomy-tag identifier in: lowercase ASCII letters,
/// digits and hyphens, and never digits alone — `4th-grade`, `homeschool` and
/// `pdf` across the read capture, against the numeric ids TPT uses for seller
/// shelves.
///
/// This is a provenance test rather than a vocabulary. Grade paths are
/// re-labelled to the target inventory without being crosswalked, so a Tes
/// year group reaches this adapter still carrying its own identifier — `2` —
/// and a tag posted under it is garbage in a live listing. Translating one
/// marketplace's grades into another's is the M6-deferred import-run
/// generalisation, and until it lands an identifier TPT cannot have issued is
/// refused rather than guessed at.
fn is_tpt_tag_slug(native: &str) -> bool {
    !native.is_empty()
        && native.bytes().any(|byte| !byte.is_ascii_digit())
        && native
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// The extensions the Markdown rendering runs under: CommonMark, plus the two
/// GFM constructs whose absence changes a body rather than leaving it alone —
/// a pipe table would reach TPT as literal pipes, a `~~cut~~` as literal
/// tildes.
///
/// The rest of what pulldown-cmark offers stays off, smart punctuation most
/// deliberately: it rewrites the seller's own quotes and dashes into other
/// characters, which edits the copy rather than rendering it. Task lists are
/// off because their rendering is an `<input>` element and what TPT's
/// rich-text field does with one is unmeasured; without the extension the
/// brackets survive as text, which is legible either way.
const MARKDOWN_EXTENSIONS: Options = Options::ENABLE_TABLES.union(Options::ENABLE_STRIKETHROUGH);

/// The body as TPT's description field takes it, which is HTML in every case.
///
/// An HTML body crosses byte-identical: TPT is the format's home and there is
/// nothing to do to it. A Markdown body is rendered under
/// [`MARKDOWN_EXTENSIONS`]. The rendering is a pure function of the bytes and
/// that constant, so the same body projects to the same HTML on every run,
/// which is what lets a projection be compared against a previous one.
///
/// Raw HTML inside a Markdown body passes through as written, because
/// CommonMark says it is HTML and the target field is an HTML field. Nothing
/// here sanitises: the body is the seller's own copy travelling from one of
/// their listings to another, and this adapter is not the boundary that would
/// decide what to strip from it.
fn body_as_html(body: &str, format: CopyFormat) -> String {
    match format {
        CopyFormat::Html => body.to_owned(),
        CopyFormat::Markdown => {
            let mut rendered = String::new();
            push_html(&mut rendered, Parser::new_ext(body, MARKDOWN_EXTENSIONS));
            rendered
        }
    }
}

/// TPT's own wire shape, rendered here rather than in the engine that seeds
/// the item. A taxonomy term whose native id parses as a number is a seller
/// shelf — TPT addresses `categories` by numeric id — and one whose native id
/// is a slug is a `taxonomyTags` member; the two are told apart by the shape
/// of the id because that is the only thing that distinguishes them on TPT's
/// own wire.
///
/// Grades are projected into the same flat slug list. TPT has no grades
/// field: `4th-grade` arrives in `taxonomyTags` beside `math` and `pdf`,
/// undifferentiated, which is what the field registry records.
///
/// A term the crosswalk left without a native id is refused. TPT addresses
/// both shelves and tags by identifiers it issued, and there is nothing to
/// send in place of one.
///
/// A body in the other format is not refused; it is rendered. TPT stores and
/// returns its description as HTML, so a Tes-sourced Markdown body posted
/// verbatim would show its `**bold**` and its `#` headings as themselves on
/// the seller's live listing, which [`body_as_html`] is here to prevent. The
/// declaration is what makes the rendering possible at all: the bytes never
/// say which of the two formats they are, and sniffing them is how a listing
/// acquires escaped markup nobody asked for.
///
/// The match on the format is total, so no arm is left over to refuse in. A
/// third body format would fail to compile at that match rather than reach a
/// runtime rejection, which is where deciding how to render it belongs.
pub fn project_fields(listing: &ProjectedListing) -> Result<FieldSet, AdapterError> {
    let body = body_as_html(&listing.body, listing.body_format);
    let price = match listing.price {
        PriceIntent::Free => json!({ "free": true }),
        // TPT's own wire carries an amount and no denomination, so an amount
        // in another currency would be posted as dollars and sold at
        // whatever that number happens to be. The inventory fixes USD, and a
        // price stated in anything else is the seller's to restate rather
        // than this adapter's to convert.
        PriceIntent::Paid(money) => {
            let CurrencyRule::Fixed(currency) = InventoryId::Tpt.currency_rule() else {
                return Err(refuse(
                    "a paid TPT listing needs a fixed currency and the inventory declares none"
                        .to_owned(),
                ));
            };
            if money.currency() != currency {
                return Err(refuse(format!(
                    "TPT sells in {} and this listing is priced in {}; the wire carries a bare \
                     amount, so posting it would sell the resource at that number of dollars",
                    currency.code(),
                    money.currency().code(),
                )));
            }
            // The currency still travels, so a mismatch stays visible to
            // whatever compares projections; `listing_from_field_set` reads
            // only the amount back.
            json!({
                "free": false,
                "minorUnits": money.minor_units(),
                "currency": format!("{:?}", money.currency()),
            })
        }
    };
    let mut tags: Vec<String> = Vec::new();
    let mut categories: Vec<String> = Vec::new();
    for term in &listing.taxonomy {
        let native = term.native_id.as_deref().ok_or_else(|| {
            refuse(format!(
                "the TPT taxonomy term {:?} carries no native identifier, and TPT addresses both \
                 its shelves and its tags by identifiers it issued",
                term.segments
            ))
        })?;
        if native.parse::<u64>().is_ok() {
            categories.push(native.to_owned());
        } else {
            tags.push(native.to_owned());
        }
    }
    let mut grades: Vec<String> = Vec::new();
    for term in &listing.grades {
        let native = term.native_id.as_deref().ok_or_else(|| {
            refuse(format!(
                "the TPT grade term {:?} carries no native identifier; grades ride the flat \
                 taxonomy-tag namespace and are addressed by slug",
                term.segments
            ))
        })?;
        if !is_tpt_tag_slug(native) {
            return Err(refuse(format!(
                "the TPT grade term {:?} carries the native identifier {native:?}, which is not \
                 the slug shape TPT issues its taxonomy tags in; a grade projected from another \
                 marketplace keeps that marketplace's own identifier, and posting it here would \
                 write it into the listing verbatim",
                term.segments
            )));
        }
        grades.push(native.to_owned());
    }
    Ok(FieldSet {
        // TPT's own wire is HTML and the projection above renders anything
        // else into it, so the declaration the seam carries is a fact of this
        // marketplace rather than of one listing.
        body_format: Some(CopyFormat::Html),
        entries: vec![
            (FieldKey::Title, listing.title.clone()),
            (FieldKey::Description, body),
            (FieldKey::Price, price.to_string()),
            (
                FieldKey::Taxonomy,
                json!({ "tags": tags, "categories": categories }).to_string(),
            ),
            (FieldKey::Grades, json!({ "tags": grades }).to_string()),
        ],
        files: listing.files.clone(),
    })
}

fn entry(fields: &FieldSet, key: FieldKey) -> Result<&str, AdapterError> {
    fields
        .entries
        .iter()
        .find(|(field, _)| *field == key)
        .map(|(_, value)| value.as_str())
        .ok_or_else(|| refuse(format!("the projection omitted {key:?}")))
}

fn json_entry(fields: &FieldSet, key: FieldKey) -> Result<Value, AdapterError> {
    serde_json::from_str(entry(fields, key)?)
        .map_err(|error| refuse(format!("the {key:?} entry is not JSON: {error}")))
}

fn string_list(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Reads the `Price` entry back into what the forms post.
///
/// The paid branch is reachable only when the projection carried
/// `PriceIntent::Paid`, whose amount is a positive one somebody set on
/// purpose; nothing here can arrive at a price by default. The currency the
/// projection named is not read: TPT's wire carries a bare decimal and its
/// currency rule is `SellerScoped` and unverified, so the denomination of the
/// founder's store is a founder question rather than something this connector
/// resolves by rendering a symbol.
fn price_from_entry(price: &Value) -> Result<ListingPrice, AdapterError> {
    match price.get("free").and_then(Value::as_bool) {
        Some(true) => Ok(ListingPrice::Free),
        Some(false) => {
            let minor_units = price
                .get("minorUnits")
                .and_then(Value::as_i64)
                .ok_or_else(|| {
                    refuse(
                        "the projection asked for a paid listing and named no amount in minor \
                         units, and a price is never inferred"
                            .to_owned(),
                    )
                })?;
            PaidPrice::new(minor_units)
                .map(ListingPrice::Paid)
                .map_err(|error| refuse(error.to_string()))
        }
        None => Err(refuse(
            "the Price entry states no free flag, so whether the listing is free is unknown"
                .to_owned(),
        )),
    }
}

/// The mirror of [`project_fields`].
pub fn listing_from_field_set(fields: &FieldSet) -> Result<TptListing, AdapterError> {
    let price = price_from_entry(&json_entry(fields, FieldKey::Price)?)?;
    let taxonomy = json_entry(fields, FieldKey::Taxonomy)?;
    let grades = json_entry(fields, FieldKey::Grades)?;
    let mut taxonomy_tags = string_list(&taxonomy, "tags");
    taxonomy_tags.extend(string_list(&grades, "tags"));
    Ok(TptListing {
        title: entry(fields, FieldKey::Title)?.to_owned(),
        description_html: entry(fields, FieldKey::Description)?.to_owned(),
        taxonomy_tags,
        category_ids: string_list(&taxonomy, "categories"),
        price,
        // Absent on the captured create, and a created product therefore
        // carries none for the publishing edit to preserve.
        tax_code: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        create_fields, dotted_path, edit_fields, entry, listing_from_field_set, project_fields,
        written_field_paths, AuthorshipDeclaration, CreateSubmission, EditSubmission, ListingPrice,
        PaidPrice, PriceError, StatusUser, TaxCode, TptListing, MIN_PRICE_MINOR_UNITS,
    };
    use crate::form::TptFormTokens;
    use crate::upload::ProcessedHandle;
    use tam_marketplace::{AdapterError, AgeSpan, NativeTerm, ProjectedListing};
    use tam_types::{
        CopyFormat, Currency, FailureCode, FieldKey, FileId, Money, PriceIntent, Timestamp, Uuid,
    };

    /// The only way to obtain a token set is to scrape a render, which is
    /// the invariant the write path depends on.
    fn tokens() -> TptFormTokens {
        crate::form::scrape_form_page(&crate::form::tests_support::create_render())
            .expect("the create render parses")
            .tokens()
            .clone()
    }

    fn listing() -> TptListing {
        TptListing {
            title: "Fractions pack".to_owned(),
            description_html: "<p>ten worksheets</p>".to_owned(),
            taxonomy_tags: vec!["math".to_owned(), "4th-grade".to_owned()],
            category_ids: vec!["1361944".to_owned()],
            price: ListingPrice::Free,
            tax_code: None,
        }
    }

    fn attested() -> AuthorshipDeclaration {
        AuthorshipDeclaration::attested("founder".to_owned(), Timestamp(1))
    }

    fn created() -> Vec<(String, String)> {
        let tokens = tokens();
        let listing = listing();
        let authorship = attested();
        let handle = ProcessedHandle::new("PROCESSEDKEY".to_owned());
        create_fields(&CreateSubmission {
            tokens: &tokens,
            listing: &listing,
            product: &handle,
            thumbs_collection_key: "COLLECTIONKEY",
            authorship: &authorship,
        })
    }

    fn value_of<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
        fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn the_create_body_opens_in_the_captured_order() {
        let fields = created();
        let opening: Vec<&str> = fields
            .iter()
            .take(6)
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(
            opening,
            vec![
                "_method",
                "data[_Token][key]",
                "data[_Csrf][csrfKey]",
                "data[_Csrf][csrfToken]",
                "data[Item][name]",
                "data[ItemDigital][product]",
            ],
            "the method override leads and the token triple follows, as both captures show"
        );
        assert_eq!(
            fields.last().map(|(name, _)| name.as_str()),
            Some("data[RevisedItem][is_post]"),
            "and the empty revision flag closes the body"
        );
    }

    #[test]
    fn the_create_body_carries_the_processed_handle_and_not_the_upload_handle() {
        let fields = created();
        assert_eq!(
            value_of(&fields, "data[ItemDigital][product]"),
            Some("PROCESSEDKEY"),
            "the form takes the queue's terminal key, never the one upload_file returned"
        );
        assert_eq!(
            value_of(&fields, "thumbs_collection_key"),
            Some("COLLECTIONKEY"),
            "the thumbnail job's collection key is the second handle the form consumes"
        );
    }

    #[test]
    fn thumbs_is_the_captured_literal_rather_than_a_derived_count() {
        assert_eq!(
            value_of(&created(), "thumbs"),
            Some("0"),
            "no source states what thumbs means, so its captured value is reproduced"
        );
    }

    #[test]
    fn a_create_leaves_the_product_a_draft() {
        assert_eq!(
            value_of(&created(), "data[Item][status_user]"),
            Some("0"),
            "draft-then-publish is safe to repeat; a create straight to live is not"
        );
        assert_eq!(
            value_of(&created(), "data[Item][free]"),
            Some("1"),
            "M7 creates free listings, and the create form is the endpoint that carries free"
        );
    }

    #[test]
    fn the_copyright_declaration_is_the_attestations_value_and_has_no_other_source() {
        let fields = created();
        assert_eq!(
            value_of(&fields, "data[ItemsProperty][copyright_declaration]"),
            Some(attested().wire_value()),
            "the wire value comes from an AuthorshipDeclaration and from nowhere else"
        );
    }

    #[test]
    fn a_create_naming_no_shelf_still_posts_the_category_name_empty() {
        let tokens = tokens();
        let authorship = attested();
        let listing = TptListing {
            category_ids: Vec::new(),
            ..listing()
        };
        let handle = ProcessedHandle::new("PROCESSEDKEY".to_owned());
        let fields = create_fields(&CreateSubmission {
            tokens: &tokens,
            listing: &listing,
            product: &handle,
            thumbs_collection_key: "COLLECTIONKEY",
            authorship: &authorship,
        });
        let posted: Vec<&str> = fields
            .iter()
            .filter(|(name, _)| name == "data[Category][Category][]")
            .map(|(_, value)| value.as_str())
            .collect();
        assert_eq!(
            posted,
            vec![""],
            "the capture with no shelf selected posts the name once, empty"
        );
        assert_eq!(
            created()
                .iter()
                .filter(|(name, _)| name == "data[Category][Category][]")
                .count(),
            1,
            "and a listing that names one shelf posts that one instead"
        );
    }

    #[test]
    fn every_taxonomy_tag_is_repeated_under_the_one_array_name() {
        let fields = created();
        let tags: Vec<&str> = fields
            .iter()
            .filter(|(name, _)| name == "data[TaxonomyTags][]")
            .map(|(_, value)| value.as_str())
            .collect();
        assert_eq!(
            tags,
            vec!["math", "4th-grade"],
            "TPT posts a repeated key rather than an indexed one"
        );
    }

    #[test]
    fn an_edit_says_the_assets_are_unchanged_rather_than_clearing_them() {
        let tokens = tokens();
        let listing = listing();
        let authorship = attested();
        let fields = edit_fields(&EditSubmission {
            tokens: &tokens,
            listing: &listing,
            thumbs: &[],
            status: StatusUser::Live,
            authorship: &authorship,
        });
        assert_eq!(
            (
                value_of(&fields, "data[ItemDigital][product]"),
                value_of(&fields, "data[ItemsProperty][product_uploaded]")
            ),
            (Some(""), Some("1")),
            "empty paired with the uploaded flag is how the form says leave the asset alone"
        );
        assert_eq!(
            value_of(&fields, "data[Item][status_user]"),
            Some("1"),
            "publishing is the edit form with the status selector moved"
        );
        assert_eq!(
            value_of(&fields, "data[Item][free]"),
            Some("1"),
            "a free listing's edit posts the flag; without it the form refuses the zero price"
        );
        assert_eq!(
            value_of(&fields, "data[ItemTaxCode][tax_code_id]"),
            Some(""),
            "a listing created without a tax code posts none back"
        );
    }

    #[test]
    fn a_free_edit_carries_the_flag_where_the_create_carries_it() {
        let tokens = tokens();
        let listing = listing();
        let authorship = attested();
        let fields = edit_fields(&EditSubmission {
            tokens: &tokens,
            listing: &listing,
            thumbs: &[],
            status: StatusUser::Draft,
            authorship: &authorship,
        });
        let money: Vec<&str> = fields
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(|name| {
                matches!(
                    *name,
                    "data[Item][description]" | "data[Item][free]" | "data[Item][price]"
                )
            })
            .collect();
        assert_eq!(
            money,
            vec![
                "data[Item][description]",
                "data[Item][free]",
                "data[Item][price]"
            ],
            "the flag sits between the description and the price, as the create posts it"
        );
    }

    #[test]
    fn an_edit_echoes_every_existing_thumbnail_handle_into_its_own_slot() {
        let page = crate::form::scrape_form_page(&crate::form::tests_support::edit_render())
            .expect("the edit render parses");
        let tokens = tokens();
        let listing = listing();
        let authorship = attested();
        let fields = edit_fields(&EditSubmission {
            tokens: &tokens,
            listing: &listing,
            thumbs: page.thumbs(),
            status: StatusUser::Live,
            authorship: &authorship,
        });
        for slot in 1..=4_u8 {
            assert_eq!(
                value_of(&fields, &format!("data[ItemDigital][thumb{slot}]")),
                Some(format!("aa/bb+cc{slot}=").as_str()),
                "an edit that drops a handle drops the product's thumbnail"
            );
            assert_eq!(
                value_of(
                    &fields,
                    &format!("data[ItemsProperty][thumb{slot}_uploaded]")
                ),
                Some("1"),
                "the flag says the slot is populated"
            );
        }
    }

    #[test]
    fn a_tax_code_travels_as_its_row_id_not_its_code() {
        let tokens = tokens();
        let authorship = attested();
        let listing = TptListing {
            tax_code: Some(TaxCode::DigitalBooks),
            ..listing()
        };
        let fields = edit_fields(&EditSubmission {
            tokens: &tokens,
            listing: &listing,
            thumbs: &[],
            status: StatusUser::Live,
            authorship: &authorship,
        });
        assert_eq!(
            value_of(&fields, "data[ItemTaxCode][tax_code_id]"),
            Some("2"),
            "the form posts the row id; the code is what the read side names"
        );
        assert_eq!(TaxCode::DigitalBooks.code(), "DB031013");
    }

    #[test]
    fn a_posted_name_maps_onto_the_unlocked_lists_dotted_vocabulary() {
        assert_eq!(dotted_path("data[Item][name]"), "Item.name");
        assert_eq!(
            dotted_path("data[Category][Category][]"),
            "Category.Category"
        );
        assert_eq!(dotted_path("data[TaxonomyTags][]"), "TaxonomyTags");
        assert_eq!(
            dotted_path("thumbs_collection_key"),
            "thumbs_collection_key"
        );
    }

    #[test]
    fn the_written_paths_exclude_the_tokens_the_unlocked_list_never_names() {
        let paths = written_field_paths();
        assert!(
            paths.contains(&"Item.name".to_owned()) && paths.contains(&"TaxonomyTags".to_owned()),
            "the probe compares what we write against what the render declares, got {paths:?}"
        );
        assert!(
            !paths
                .iter()
                .any(|path| path.contains("_Token") || path == "_method"),
            "SecurityComponent never lists its own tokens among the unlocked names, got {paths:?}"
        );
    }

    fn projected(price: PriceIntent) -> ProjectedListing {
        ProjectedListing {
            title: "Fractions".to_owned(),
            body: "<p>ten</p>".to_owned(),
            price,
            taxonomy: vec![
                NativeTerm {
                    native_id: Some("math".to_owned()),
                    segments: vec!["Math".to_owned()],
                },
                NativeTerm {
                    native_id: Some("1361944".to_owned()),
                    segments: vec!["My shelf".to_owned()],
                },
            ],
            grades: vec![NativeTerm {
                native_id: Some("4th-grade".to_owned()),
                segments: vec!["Grade 4".to_owned()],
            }],
            ages: Some(AgeSpan {
                low_years: 9,
                high_years: 10,
            }),
            files: vec![FileId(Uuid([1; 16]))],
            body_format: CopyFormat::Html,
            natives: Vec::new(),
        }
    }

    fn described(body: &str, format: CopyFormat) -> String {
        let fields = project_fields(&ProjectedListing {
            body: body.to_owned(),
            body_format: format,
            ..projected(PriceIntent::Free)
        })
        .expect("a body in either declared format projects");
        entry(&fields, FieldKey::Description)
            .expect("the projection carries a description")
            .to_owned()
    }

    /// O.16, settled the other way. TPT stores and returns its description as
    /// HTML and a Tes body is Markdown, so the projection renders rather than
    /// leaving the seller's `**bold**` to be read as itself on a live
    /// listing.
    #[test]
    fn a_markdown_body_is_rendered_into_the_html_field() {
        let html = described(
            "## Fractions\n\n**bold** text\n\n- one\n- two\n",
            CopyFormat::Markdown,
        );
        assert!(
            html.contains("<h2>Fractions</h2>"),
            "a heading renders as a heading element, got {html}"
        );
        assert!(
            html.contains("<strong>bold</strong>"),
            "emphasis renders as markup, got {html}"
        );
        assert!(
            html.contains("<ul>") && html.contains("<li>one</li>"),
            "a list renders as a list, got {html}"
        );
        assert!(
            !html.contains("**") && !html.contains("## "),
            "no source syntax survives to be read literally, got {html}"
        );
    }

    /// The declaration decides, and TPT is HTML's home: a body already in the
    /// target format reaches the field as the bytes it arrived as, with no
    /// parse-and-reserialise round trip in the way.
    #[test]
    fn an_html_body_crosses_byte_identical() {
        let source = "<p>ten worksheets</p>\n<!-- editor -->";
        assert_eq!(
            described(source, CopyFormat::Html),
            source,
            "an HTML body is passed through rather than re-rendered"
        );
    }

    /// The rendering is a pure function of the bytes and `MARKDOWN_EXTENSIONS`.
    /// A projection that varied per run could not be compared against the
    /// previous one, which is what tells a sync there is nothing to write.
    #[test]
    fn the_rendering_is_deterministic() {
        let source = "# One\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n~~cut~~ and `code`\n";
        assert_eq!(
            described(source, CopyFormat::Markdown),
            described(source, CopyFormat::Markdown),
            "the same body renders to the same bytes"
        );
    }

    /// The extension set is the choice this conversion turns on, so it is
    /// pinned by behaviour rather than by reading the constant back. Tables
    /// and strikethrough are on because their absence changes a body;
    /// smart punctuation is off because its presence edits one.
    #[test]
    fn the_extension_set_renders_gfm_and_leaves_the_sellers_punctuation_alone() {
        let table = described("| a | b |\n|---|---|\n| 1 | 2 |\n", CopyFormat::Markdown);
        assert!(
            table.contains("<table>") && table.contains("<td>1</td>"),
            "a pipe table renders as a table rather than as literal pipes, got {table}"
        );
        let struck = described("~~cut~~\n", CopyFormat::Markdown);
        assert!(
            struck.contains("<del>cut</del>"),
            "strikethrough renders as markup rather than as literal tildes, got {struck}"
        );
        let punctuation = described("She said \"no\" -- twice...\n", CopyFormat::Markdown);
        assert!(
            punctuation.contains("She said \"no\" -- twice..."),
            "the seller's own quotes, dashes and dots survive as the characters they typed, \
             got {punctuation}"
        );
        assert!(
            !punctuation.contains('\u{201c}')
                && !punctuation.contains('\u{2014}')
                && !punctuation.contains('\u{2026}'),
            "smart punctuation is off, so no curly quote, em dash or ellipsis is substituted \
             in, got {punctuation}"
        );
    }

    /// The rendered body is a projected body, so the projection's own parser
    /// reads it back as the description TPT would be posted.
    #[test]
    fn a_rendered_body_reaches_the_submission_the_parser_builds() {
        let fields = project_fields(&ProjectedListing {
            body: "**bold**\n".to_owned(),
            body_format: CopyFormat::Markdown,
            ..projected(PriceIntent::Free)
        })
        .expect("a Markdown body projects");
        let listing =
            listing_from_field_set(&fields).expect("the submit parses its own projection");
        assert_eq!(
            listing.description_html,
            entry(&fields, FieldKey::Description).expect("the projection carries a description"),
            "the submission carries the rendered body and not the Markdown source"
        );
        assert_eq!(
            fields.body_format,
            Some(CopyFormat::Html),
            "the projected body declares the format TPT stores, whatever the source declared"
        );
    }

    #[test]
    fn the_projection_and_its_parser_are_one_contract() {
        let fields =
            project_fields(&projected(PriceIntent::Free)).expect("a free listing projects");
        let listing =
            listing_from_field_set(&fields).expect("the submit parses its own projection");
        assert_eq!(
            listing.taxonomy_tags,
            vec!["math".to_owned(), "4th-grade".to_owned()],
            "grades ride the flat tag namespace, appended after the subject tags"
        );
        assert_eq!(
            listing.category_ids,
            vec!["1361944".to_owned()],
            "a numeric native id is a seller shelf, which is the only thing TPT numbers"
        );
        assert_eq!(fields.files.len(), 1, "the file list crosses untouched");
    }

    /// G-O6, settled: TPT sells in USD and offers the seller no other
    /// currency. Its wire carries a bare amount, so a pound price posted here
    /// would sell the resource at that many dollars; the seller restates the
    /// target price rather than this adapter inventing an exchange rate.
    #[test]
    fn a_price_in_another_currency_than_the_one_tpt_sells_in_is_refused() {
        let money = Money::new(300, Currency::Gbp).expect("a positive amount is money");
        let refused = project_fields(&projected(PriceIntent::Paid(money)));
        let Err(AdapterError::Rejected { detail, .. }) = refused else {
            panic!("a price TPT cannot denominate is a rejection, got {refused:?}");
        };
        assert!(
            detail.0.contains("USD") && detail.0.contains("GBP"),
            "the refusal names both currencies, got {}",
            detail.0
        );
    }

    #[test]
    fn a_paid_projection_posts_the_captured_paid_shape_rather_than_the_free_one() {
        let money = Money::new(300, Currency::Usd).expect("a positive amount is money");
        let fields =
            project_fields(&projected(PriceIntent::Paid(money))).expect("a paid listing projects");
        let listing = listing_from_field_set(&fields).expect("a deliberate price parses back");
        let tokens = tokens();
        let authorship = attested();
        let handle = ProcessedHandle::new("PROCESSEDKEY".to_owned());
        let created = create_fields(&CreateSubmission {
            tokens: &tokens,
            listing: &listing,
            product: &handle,
            thumbs_collection_key: "COLLECTIONKEY",
            authorship: &authorship,
        });
        assert_eq!(
            (
                value_of(&created, "data[Item][free]"),
                value_of(&created, "data[Item][price]"),
                value_of(&created, "data[Item][license_price]"),
                value_of(&created, "data[Item][discountprice]"),
            ),
            (Some("0"), Some("3.00"), Some("2.70"), Some("0")),
            "the captured paid edit is 3.00 against 2.70, and the free create's flag inverts"
        );
        let edited = edit_fields(&EditSubmission {
            tokens: &tokens,
            listing: &listing,
            thumbs: &[],
            status: StatusUser::Live,
            authorship: &authorship,
        });
        assert_eq!(
            (
                value_of(&edited, "data[Item][price]"),
                value_of(&edited, "data[Item][license_price]"),
            ),
            (Some("3.00"), Some("2.70")),
            "the edit carries the same amounts, which is the shape the capture recorded"
        );
        assert!(
            !edited.iter().any(|(name, _)| name == "data[Item][free]"),
            "and a priced edit still omits free, which is what the captured paid edit posts"
        );
    }

    #[test]
    fn the_licence_price_is_ninety_percent_and_says_how_it_rounds() {
        let paid = |minor| PaidPrice::new(minor).map(|price| price.licence_amount().to_owned());
        assert_eq!(
            paid(300).as_deref(),
            Ok("2.70"),
            "the one captured pair, where every rounding rule agrees"
        );
        assert_eq!(
            paid(199).as_deref(),
            Ok("1.79"),
            "179.1 minor units rounds to 179; no capture settles the tie, so the rule is stated"
        );
        assert_eq!(
            paid(95).as_deref(),
            Ok("0.86"),
            "85.5 minor units is the half case itself, and half goes up"
        );
    }

    #[test]
    fn a_price_under_the_forms_own_minimum_is_refused_before_a_submit_opens() {
        assert_eq!(
            PaidPrice::new(94).err(),
            Some(PriceError::BelowMinimum { minor_units: 94 }),
            "the form states min_price 0.95, and a submit it refuses comes back as an ambiguity"
        );
        assert!(
            PaidPrice::new(MIN_PRICE_MINOR_UNITS).is_ok(),
            "the minimum itself is a price"
        );
        let mut fields =
            project_fields(&projected(PriceIntent::Free)).expect("a free listing projects");
        fields.entries.retain(|(key, _)| *key != FieldKey::Price);
        fields.entries.push((
            FieldKey::Price,
            r#"{"free":false,"minorUnits":10,"currency":"Usd"}"#.to_owned(),
        ));
        let refused = listing_from_field_set(&fields);
        let Err(AdapterError::Rejected { code, detail }) = refused else {
            panic!("a below-minimum price must not reach the form, got {refused:?}");
        };
        assert_eq!(code, FailureCode::UploadRejected);
        assert!(
            detail.0.contains("minimum"),
            "the refusal names the floor it failed, got {detail:?}"
        );
    }

    #[test]
    fn a_paid_projection_without_an_amount_is_refused_rather_than_freed() {
        let mut fields =
            project_fields(&projected(PriceIntent::Free)).expect("a free listing projects");
        fields.entries.retain(|(key, _)| *key != FieldKey::Price);
        fields
            .entries
            .push((FieldKey::Price, r#"{"free":false}"#.to_owned()));
        let refused = listing_from_field_set(&fields);
        let Err(AdapterError::Rejected { detail, .. }) = refused else {
            panic!("an amountless paid listing has nothing to post, got {refused:?}");
        };
        assert!(
            detail.0.contains("never inferred"),
            "the refusal says a price is never inferred, got {detail:?}"
        );
    }

    #[test]
    fn a_term_without_a_native_identifier_is_refused_before_an_attempt_opens() {
        let mut listing = projected(PriceIntent::Free);
        listing.taxonomy.push(NativeTerm {
            native_id: None,
            segments: vec!["Uncrosswalked".to_owned()],
        });
        let refused = project_fields(&listing);
        let Err(AdapterError::Rejected { detail, .. }) = refused else {
            panic!("there is nothing to send in place of an identifier, got {refused:?}");
        };
        assert!(
            detail.0.contains("Uncrosswalked"),
            "the refusal names the term that could not be projected, got {detail:?}"
        );
    }

    #[test]
    fn a_grade_identifier_tpt_never_issued_is_refused_rather_than_posted_as_a_tag() {
        let mut listing = projected(PriceIntent::Free);
        // A Tes year group as `project_listing` hands it over: the grade path
        // is re-labelled to the target inventory and its native id passes
        // through uncrosswalked, so this is what actually arrives.
        listing.grades.push(NativeTerm {
            native_id: Some("2".to_owned()),
            segments: vec!["Year 2".to_owned()],
        });
        let refused = project_fields(&listing);
        let Err(AdapterError::Rejected { code, detail }) = refused else {
            panic!("a foreign grade identifier must not reach the wire, got {refused:?}");
        };
        assert_eq!(code, FailureCode::UploadRejected);
        assert!(
            detail.0.contains("Year 2") && detail.0.contains("slug shape"),
            "the refusal names the term and the provenance gap, got {detail:?}"
        );
    }

    #[test]
    fn a_tpt_issued_grade_slug_still_rides_the_flat_tag_namespace() {
        let fields = project_fields(&projected(PriceIntent::Free)).expect("it projects");
        let listing = listing_from_field_set(&fields).expect("it parses back");
        assert!(
            listing.taxonomy_tags.contains(&"4th-grade".to_owned()),
            "the refusal is about provenance, not about grades, got {:?}",
            listing.taxonomy_tags
        );
    }

    #[test]
    fn a_field_set_missing_a_key_refuses_rather_than_defaulting() {
        let mut fields = project_fields(&projected(PriceIntent::Free)).expect("it projects");
        fields.entries.retain(|(key, _)| *key != FieldKey::Title);
        assert!(
            matches!(
                listing_from_field_set(&fields),
                Err(AdapterError::Rejected { .. })
            ),
            "an absent title is a refusal, never an empty one"
        );
    }
}
