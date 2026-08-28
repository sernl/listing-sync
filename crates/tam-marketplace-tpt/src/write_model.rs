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

use serde_json::{json, Value};
use tam_marketplace::{AdapterError, FieldSet, ProjectedListing};
use tam_types::{FailureCode, FailureDetail, FieldKey, PriceIntent, Timestamp};

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
    /// carried `<p>` tags and an editor comment.
    pub description_html: String,
    /// The flat slug namespace: grade, subject, audience, resource type and
    /// file format, undifferentiated, exactly as the read side returns them.
    pub taxonomy_tags: Vec<String>,
    /// Seller-owned shelves, addressed by their numeric ids.
    pub category_ids: Vec<String>,
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
    fields.push(field(names::FREE, "1"));
    fields.push(field(names::PRICE, "0"));
    fields.push(field(names::DISCOUNTPRICE, "0"));
    fields.push(field(names::LICENSE_PRICE, "0"));
    fields.push(field(names::STATUS_USER, StatusUser::Draft.as_str()));
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

/// The edit body: the 48 fields the captured `POST /itemsDigital/editNext/{id}`
/// carried, in its order.
///
/// An edit is a full replace, so every name is posted even where its value is
/// empty. Empty in `product`, `preview` and `videopreview` paired with an
/// `*_uploaded` flag of `1` is how the form says "this asset is unchanged" —
/// the captured edit re-uploaded nothing and touched no S3 host.
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
    fields.push(field(names::PRICE, "0"));
    fields.push(field(names::DISCOUNTPRICE, "0"));
    fields.push(field(names::LICENSE_PRICE, "0"));
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
pub fn project_fields(listing: &ProjectedListing) -> Result<FieldSet, AdapterError> {
    let price = match listing.price {
        PriceIntent::Free => json!({ "free": true }),
        // M7 ships free create only. The projection still renders the intent
        // rather than silently freeing it, and `listing_from_field_set`
        // refuses it by name, mirroring the Tes paid-licence refusal.
        PriceIntent::Paid(money) => json!({
            "free": false,
            "minorUnits": money.minor_units(),
            "currency": format!("{:?}", money.currency()),
        }),
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
        grades.push(native.to_owned());
    }
    Ok(FieldSet {
        entries: vec![
            (FieldKey::Title, listing.title.clone()),
            (FieldKey::Description, listing.body.clone()),
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

/// The mirror of [`project_fields`]. Refuses a paid projection by name: no
/// paid create is captured, and `min_price: 0.95` in the form's own bootstrap
/// carries no currency symbol and no ISO code, so this connector cannot state
/// what a price it posted would be denominated in.
pub fn listing_from_field_set(fields: &FieldSet) -> Result<TptListing, AdapterError> {
    let price = json_entry(fields, FieldKey::Price)?;
    if price.get("free").and_then(Value::as_bool) != Some(true) {
        let currency = price
            .get("currency")
            .and_then(Value::as_str)
            .unwrap_or("an unnamed currency");
        return Err(refuse(format!(
            "TPT create is free-listing only in M7: the projection asked for a paid listing in \
             {currency}, no paid create is captured, and the form's own min_price of 0.95 names \
             no currency, so the amount cannot be stated on the wire"
        )));
    }
    let taxonomy = json_entry(fields, FieldKey::Taxonomy)?;
    let grades = json_entry(fields, FieldKey::Grades)?;
    let mut taxonomy_tags = string_list(&taxonomy, "tags");
    taxonomy_tags.extend(string_list(&grades, "tags"));
    Ok(TptListing {
        title: entry(fields, FieldKey::Title)?.to_owned(),
        description_html: entry(fields, FieldKey::Description)?.to_owned(),
        taxonomy_tags,
        category_ids: string_list(&taxonomy, "categories"),
        // Absent on the captured create, and a created product therefore
        // carries none for the publishing edit to preserve.
        tax_code: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        create_fields, dotted_path, edit_fields, listing_from_field_set, project_fields,
        written_field_paths, AuthorshipDeclaration, CreateSubmission, EditSubmission, StatusUser,
        TaxCode, TptListing,
    };
    use crate::form::TptFormTokens;
    use crate::upload::ProcessedHandle;
    use tam_marketplace::{AdapterError, AgeSpan, NativeTerm, ProjectedListing};
    use tam_types::{Currency, FailureCode, FieldKey, FileId, Money, PriceIntent, Timestamp, Uuid};

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
        assert!(
            !fields.iter().any(|(name, _)| name == "data[Item][free]"),
            "the edit form omits free entirely, and adding it would be a field TPT never sent"
        );
        assert_eq!(
            value_of(&fields, "data[ItemTaxCode][tax_code_id]"),
            Some(""),
            "a listing created without a tax code posts none back"
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
        }
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

    #[test]
    fn a_paid_projection_is_refused_by_name_rather_than_silently_freed() {
        let money = Money::new(300, Currency::Usd).expect("a positive amount is money");
        let fields =
            project_fields(&projected(PriceIntent::Paid(money))).expect("a paid listing projects");
        let refused = listing_from_field_set(&fields);
        let Err(AdapterError::Rejected { code, detail }) = refused else {
            panic!("M7 is free-create only, so a paid seed must refuse, got {refused:?}");
        };
        assert_eq!(code, FailureCode::UploadRejected);
        assert!(
            detail.0.contains("Usd") && detail.0.contains("min_price"),
            "the refusal names the currency asked for and why it cannot be honoured, got {detail:?}"
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
