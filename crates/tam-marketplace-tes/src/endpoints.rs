//! Typed builders and parsers for the M0-confirmed endpoints, recorded in
//! docs/design/decisions.md: create, metadata, presign-and-confirm, the S3
//! form POST, publish (which requires a licence on the draft first), read and
//! delete. Builders return seam `HttpRequest` values so cassettes and the
//! live transport are interchangeable.

use base64::Engine;
use serde_json::{json, Value};
use tam_marketplace::transport::{FilePart, HttpRequest, Method, RequestAuth, RequestBody};
use tam_marketplace::RemoteListingId;
use tam_types::CopyFormat;

pub const ORIGIN: &str = "https://www.tes.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DraftId(pub i64);

impl DraftId {
    /// The canonical resource identity, and the one thing every path that can
    /// bind a mapping must agree on: this string becomes the bind's remote-id
    /// columns, `classify_bind` compares them by exact equality, and
    /// `mapping_one_bound_url` indexes them, so a second spelling of one
    /// resource makes a later write report `DivergentLanding` against the
    /// mapping it had just successfully written.
    #[must_use]
    pub fn canonical_url(self) -> String {
        format!("{ORIGIN}/api/v2/resources/{}", self.0)
    }

    #[must_use]
    pub fn remote(self) -> RemoteListingId {
        RemoteListingId::Tes {
            url: self.canonical_url(),
        }
    }
}

/// The catalogue and the import manifest both carry a resource as its bare
/// numeric id, which is how a platform-agnostic importer names one without
/// naming this crate.
impl From<i64> for DraftId {
    fn from(resource: i64) -> Self {
        Self(resource)
    }
}

/// The licence values the API validates. Free resources use the Creative
/// Commons family; `TES-PAID` requires a price and is refused without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TesLicence {
    CcBy,
    CcBySa,
    CcByNd,
    TesPaid,
}

impl TesLicence {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CcBy => "CC-BY",
            Self::CcBySa => "CC-BY-SA",
            Self::CcByNd => "CC-BY-ND",
            Self::TesPaid => "TES-PAID",
        }
    }
}

/// Every licence `RefdataStore.licences` holds, which is the read side and a
/// strict superset of [`TesLicence`], the writable four.
///
/// `GET /api/refdata/v2/licences` returns the first five and omits the legacy
/// pair; the editor rewrites `TES-V1` and `TES-V2` to `CC-BY-SA` on load, so
/// both read back on older resources and neither is offered on write.
/// `TES-PAID-SCHOOL` is the school tier, real on read and not something this
/// adapter writes. An import that models four of seven refuses three real
/// licences as unrecognised and drops the seller's grant on the floor.
///
/// Restated here rather than imported from the registry: the pure core must
/// not depend on an adapter crate, so the dependency runs the other way and
/// this is the adapter's own transcription. `the_seven_tokens_agree_with_the_polled_vocabulary`
/// binds it to the JSON rather than trusting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadLicence {
    CcBy,
    CcByNd,
    CcBySa,
    TesPaid,
    TesPaidSchool,
    TesV1,
    TesV2,
}

impl ReadLicence {
    pub const ALL: [Self; 7] = [
        Self::CcBy,
        Self::CcByNd,
        Self::CcBySa,
        Self::TesPaid,
        Self::TesPaidSchool,
        Self::TesV1,
        Self::TesV2,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CcBy => "CC-BY",
            Self::CcByNd => "CC-BY-ND",
            Self::CcBySa => "CC-BY-SA",
            Self::TesPaid => "TES-PAID",
            Self::TesPaidSchool => "TES-PAID-SCHOOL",
            Self::TesV1 => "TES-V1",
            Self::TesV2 => "TES-V2",
        }
    }

    /// `None` rather than a guess: an unlisted token is a licence this
    /// adapter has never seen, and reading it as free would state a rights
    /// grant nobody made.
    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|row| row.as_str() == token)
    }

    /// Whether the licence gates the write on a price. The API refuses a
    /// Creative Commons value with a price and refuses a paid one without,
    /// so this is the branch a price intent is read off.
    #[must_use]
    pub const fn is_paid(self) -> bool {
        match self {
            Self::TesPaid | Self::TesPaidSchool => true,
            Self::CcBy | Self::CcByNd | Self::CcBySa | Self::TesV1 | Self::TesV2 => false,
        }
    }
}

/// The three Creative Commons licences, which are the free tier: the API
/// refuses a price against one of them and refuses `TES-PAID` without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeLicence {
    CcBy,
    CcBySa,
    CcByNd,
}

impl FreeLicence {
    #[must_use]
    pub const fn licence(self) -> TesLicence {
        match self {
            Self::CcBy => TesLicence::CcBy,
            Self::CcBySa => TesLicence::CcBySa,
            Self::CcByNd => TesLicence::CcByNd,
        }
    }
}

/// A price in the minor units the API's own integer carries: the captured
/// publish body reads `"price": 500` for GBP 5.00. The denomination is not a
/// per-listing datum here — Tes fixes the currency by inventory — so this
/// carries the integer alone, exactly as the wire does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TesPrice(i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAPrice(pub i64);

impl core::fmt::Display for NotAPrice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "a Tes price is a positive number of minor units, and {} is not one",
            self.0
        )
    }
}

impl core::error::Error for NotAPrice {}

impl TesPrice {
    /// A positive amount. Zero is not a price on this marketplace either; it
    /// is a Creative Commons licence, which is a different listing.
    pub const fn new(minor_units: i64) -> Result<Self, NotAPrice> {
        if minor_units <= 0 {
            return Err(NotAPrice(minor_units));
        }
        Ok(Self(minor_units))
    }

    #[must_use]
    pub const fn minor_units(self) -> i64 {
        self.0
    }
}

/// The licence and the price as one value, so a `TES-PAID` listing without a
/// price and a Creative Commons listing carrying one are both unrepresentable
/// rather than merely validated. `TES-PAID` beside `price` in minor units is
/// the pairing the captured publish body carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TesPricing {
    Free(FreeLicence),
    Paid(TesPrice),
}

impl TesPricing {
    #[must_use]
    pub const fn licence(self) -> TesLicence {
        match self {
            Self::Free(free) => free.licence(),
            Self::Paid(_) => TesLicence::TesPaid,
        }
    }

    #[must_use]
    pub const fn price(self) -> Option<TesPrice> {
        match self {
            Self::Free(_) => None,
            Self::Paid(price) => Some(price),
        }
    }
}

/// The metadata the draft endpoint accepts, in the field names the API uses.
/// Category and age identifiers arrive already projected; the taxonomy hub
/// (M1g) owns how they are derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TesListing {
    pub title: String,
    pub description_raw: String,
    /// Which of the two formats `descriptionRawType` declares for the bytes
    /// above. Tes takes either: the 2026-08-29 probe posted `html` to a live
    /// draft, the API echoed the type back and the markup round-tripped
    /// byte-intact, so a TPT-sourced body travels as itself rather than being
    /// refused or converted.
    pub description_format: CopyFormat,
    pub category_ids: Vec<i64>,
    pub age_channel: TesAges,
    pub ages: Vec<i64>,
    pub main_type: Option<i64>,
    /// Which `ageRanges` band buyers see, as a band id and not an age: the
    /// capture pairs `mainAge: 6` with `ages: [16,17,18]`, and the resource
    /// page renders that band's own label alone.
    ///
    /// Absent where the declaration derives no ages at all -- the
    /// not-applicable band, or no band -- because the registry declares the
    /// field optional and a zero here names a real band as surely as
    /// `mainType: 0` named a real resource type.
    pub main_age: Option<i64>,
    pub pricing: TesPricing,
}

/// The age ids a Tes listing declares, posted as `ageRanges`.
///
/// The uploader picks the field by country -- `ageResourceFieldName =
/// country === "GB" ? "ageRanges" : "yearGroups"` -- and the two carry ids
/// from different vocabularies: id 2 names the 5-7 age band in one and
/// Reception in the other. Tes is one inventory on the `ageRanges` branch
/// since 2026-09-12, and `yearGroups` travels empty because the captured
/// draft body carries the key (probe 02 line 15).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TesAges(Vec<i64>);

impl TesAges {
    /// The wire field these ids answer.
    pub const FIELD: &'static str = "ageRanges";

    #[must_use]
    pub const fn new(ids: Vec<i64>) -> Self {
        Self(ids)
    }

    #[must_use]
    pub fn ids(&self) -> &[i64] {
        &self.0
    }

    /// The two derived age fields this declaration posts.
    ///
    /// The 2026-08-29 capture fixes the rule on three independent states
    /// across two recordings: `ages` is the sorted union of the declared
    /// bands' `humanAges` and `mainAge` names one of the bands. A contiguous
    /// `low..=high` fill agrees with the union only where the bands are
    /// adjacent; for the captured `[2, 6]` the union is `{5,6,7,16,17,18}`
    /// where a fill would post everything from 5 to 18.
    pub fn derived_ages(&self) -> Result<DerivedAges, UnknownAgeBand> {
        let mut ages: Vec<i64> = Vec::new();
        let mut main_age = None;
        for id in &self.0 {
            let band = AGE_BANDS
                .iter()
                .find(|band| band.id == *id)
                .ok_or(UnknownAgeBand(*id))?;
            if band.human_ages.is_empty() {
                continue;
            }
            // The band buyers see. Which band that is, where a seller
            // declared several, is theirs to elect; until the election
            // lands the first declared band with ages is the one named,
            // and never the not-applicable row, which declares none.
            main_age.get_or_insert(band.id);
            ages.extend_from_slice(band.human_ages);
        }
        ages.sort_unstable();
        ages.dedup();
        Ok(DerivedAges { ages, main_age })
    }
}

/// The literal `ages` array a declaration posts and the band `mainAge` names.
/// Both are absent together: `mainAge` names what `ages` is derived from, and
/// a declaration deriving no ages -- the not-applicable band, or none at all
/// -- states neither rather than an empty list beside age zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedAges {
    pub ages: Vec<i64>,
    pub main_age: Option<i64>,
}

/// A grade id the `ageRanges` vocabulary does not hold. The crosswalk seeds
/// these ids from that vocabulary, so one that is not in it is a seeding
/// fault; refusing names it rather than posting a band Tes will read as
/// something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownAgeBand(pub i64);

/// One row of the uploader's `ageRanges` vocabulary, with the `humanAges` Tes
/// itself publishes for it, transcribed from `docs/design/data/tes-vocabulary
/// .json` and checked against it below.
///
/// Band 6 is the one worth stating plainly: `16+` is not half-open on the
/// wire. Tes closes it at `{16, 17, 18}` and its picker offers nothing above
/// 18, so a 16+ listing posts real ages rather than omitting the pair.
struct AgeBand {
    id: i64,
    human_ages: &'static [i64],
}

const AGE_BANDS: [AgeBand; 7] = [
    AgeBand {
        id: 1,
        human_ages: &[3, 4, 5],
    },
    AgeBand {
        id: 2,
        human_ages: &[5, 6, 7],
    },
    AgeBand {
        id: 3,
        human_ages: &[7, 8, 9, 10, 11],
    },
    AgeBand {
        id: 4,
        human_ages: &[11, 12, 13, 14],
    },
    AgeBand {
        id: 5,
        human_ages: &[14, 15, 16],
    },
    AgeBand {
        id: 6,
        human_ages: &[16, 17, 18],
    },
    AgeBand {
        id: 7,
        human_ages: &[],
    },
];

#[must_use]
pub fn create_draft_request() -> HttpRequest {
    HttpRequest::post_json(format!("{ORIGIN}/api/v2/resources"), json!({}))
}

/// The title prefix marking every automation-created artefact disposable, so
/// a human can sweep the dashboard for strays; the rehearsal invariants in
/// the runbook bound how many may exist at once.
pub const ZZ_TITLE_PREFIX: &str = "ZZ-SMOKE-DELETE-ME";

/// The schema probe's payload. The live API omits null scalar keys from a
/// draft's JSON, so an empty draft cannot witness the written-field set; the
/// probe writes every field first and asserts the read-back. The ids are the
/// cassette-proven ones from the recorded captures.
#[must_use]
pub fn probe_listing() -> TesListing {
    TesListing {
        title: ZZ_TITLE_PREFIX.to_owned(),
        description_raw: "Automated schema probe. **Delete me.**".to_owned(),
        description_format: CopyFormat::Markdown,
        category_ids: vec![1_000_448],
        age_channel: TesAges::new(vec![4]),
        ages: vec![11, 12, 13, 14],
        main_type: Some(99_009),
        main_age: Some(4),
        pricing: TesPricing::Free(FreeLicence::CcBy),
    }
}

/// The `descriptionRawType` token one body format posts under.
///
/// Both are live-proven: the M0 captures wrote `md` throughout, and the
/// 2026-08-29 probe wrote `html` to a draft, read the type back unchanged and
/// found the markup byte-intact. So the type is a function of the body rather
/// than the constant it used to be, and a TPT-sourced listing crosses into
/// Tes as itself.
#[must_use]
pub const fn raw_type(format: CopyFormat) -> &'static str {
    match format {
        CopyFormat::Markdown => "md",
        CopyFormat::Html => "html",
    }
}

/// The format a `descriptionRawType` token names, or `None` for a token this
/// adapter does not model -- which a read refuses rather than reading as
/// markdown, because guessing the format of bytes is the failure the
/// declaration exists to prevent.
#[must_use]
pub fn format_from_raw_type(token: &str) -> Option<CopyFormat> {
    match token {
        "md" => Some(CopyFormat::Markdown),
        "html" => Some(CopyFormat::Html),
        _ => None,
    }
}

/// The listing metadata in the API's own field names, which the draft POST
/// and the publish POST both carry — the captured publish body restates every
/// field the draft holds rather than referring to it.
///
/// `price` accompanies the `TES-PAID` licence and is absent otherwise, which
/// is the pairing [`TesPricing`] makes unrepresentable to get wrong.
fn metadata_body(listing: &TesListing) -> Value {
    let categories: Vec<Value> = listing
        .category_ids
        .iter()
        .map(|category| json!({ "id": category }))
        .collect();
    // `yearGroups` travels empty rather than absent: the captured draft body
    // carries the key, and the ids this adapter writes are age bands.
    let mut body = json!({
        "title": listing.title,
        "descriptionRaw": listing.description_raw,
        "descriptionRawType": raw_type(listing.description_format),
        "categories": categories,
        "ageRanges": listing.age_channel.ids(),
        "yearGroups": Vec::<i64>::new(),
        "licence": listing.pricing.licence().as_str(),
    });
    // The registry declares `mainType` optional, so an unprojected resource
    // type omits the key. It used to be sent as zero, which is a real Tes
    // type and named one on every listing we ever created.
    if let Some(main_type) = listing.main_type {
        body["mainType"] = json!(main_type);
    }
    // The same rule for the age fields, which the registry also declares
    // optional. They travel together because they are one derivation: the
    // band buyers see, the union of every declared band's ages, and the other
    // band where the seller declared a second one. The draft POST carries all
    // three -- entry 44 of the 2026-08-29 capture is a draft and holds
    // `additionalAge` -- so the publish restates them rather than adding one.
    if let Some(main_age) = listing.main_age {
        body["ages"] = json!(listing.ages);
        body["mainAge"] = json!(main_age);
        if let Some(additional) = additional_age(listing) {
            body["additionalAge"] = json!(additional);
        }
    }
    if let Some(price) = listing.pricing.price() {
        body["price"] = json!(price.minor_units());
    }
    body
}

#[must_use]
pub fn set_metadata_request(id: DraftId, listing: &TesListing) -> HttpRequest {
    HttpRequest::post_json(
        format!("{ORIGIN}/api/v2/resources/{}/draft", id.0),
        metadata_body(listing),
    )
}

/// The age range the body carries beside `mainAge`: the other band the
/// declaration names. Absent from a declaration naming only its main one, and
/// omitted rather than invented when there is none.
///
/// Absent too when the declaration states no main age at all, because an
/// additional age names what it is additional to.
fn additional_age(listing: &TesListing) -> Option<i64> {
    let main = listing.main_age?;
    listing
        .age_channel
        .ids()
        .iter()
        .copied()
        .find(|range| *range != main)
}

#[must_use]
pub fn presign_request(id: DraftId, file_name: &str, temp_id: &str) -> HttpRequest {
    HttpRequest::post_json(
        format!("{ORIGIN}/api/resources/v3/draft/{}/attachment", id.0),
        json!([{
            "name": file_name,
            "tempId": temp_id,
            "previewOption": 0,
        }]),
    )
}

/// What the presign response yields once decoded: the bucket URL, the exact
/// form fields the policy signs plus the starts-with fields it constrains,
/// and the attachment object to echo back on confirm.
#[derive(Debug, Clone, PartialEq)]
pub struct PresignedUpload {
    pub s3_url: String,
    pub fields: Vec<(String, String)>,
    pub attachment: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresignParseError {
    Shape(String),
    Policy(String),
}

impl core::fmt::Display for PresignParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Shape(detail) => write!(f, "presign response shape: {detail}"),
            Self::Policy(detail) => write!(f, "presign policy: {detail}"),
        }
    }
}

impl core::error::Error for PresignParseError {}

/// Decodes the base64 S3 POST policy to recover the bucket and the
/// starts-with field names, exactly as measured in M0: the policy is the
/// only place the bucket appears.
fn policy_info(policy_b64: &str) -> Result<(String, Vec<String>), PresignParseError> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(policy_b64)
        .map_err(|error| PresignParseError::Policy(format!("not base64: {error}")))?;
    let value: Value = serde_json::from_slice(&raw)
        .map_err(|error| PresignParseError::Policy(format!("not JSON: {error}")))?;
    let mut bucket = None;
    let mut starts_with = Vec::new();
    let conditions = value
        .get("conditions")
        .and_then(Value::as_array)
        .ok_or_else(|| PresignParseError::Policy("no conditions array".to_owned()))?;
    for condition in conditions {
        if let Some(name) = condition.get("bucket").and_then(Value::as_str) {
            bucket = Some(name.to_owned());
        }
        if let Some(triple) = condition.as_array() {
            if triple.first().and_then(Value::as_str) == Some("starts-with") {
                if let Some(field) = triple.get(1).and_then(Value::as_str) {
                    starts_with.push(field.trim_start_matches('$').to_owned());
                }
            }
        }
    }
    let bucket =
        bucket.ok_or_else(|| PresignParseError::Policy("no bucket condition".to_owned()))?;
    Ok((bucket, starts_with))
}

pub fn parse_presign(
    body: &Value,
    file_name: &str,
    content_type: &str,
) -> Result<PresignedUpload, PresignParseError> {
    let attachment = body
        .get(0)
        .cloned()
        .ok_or_else(|| PresignParseError::Shape("empty presign array".to_owned()))?;
    let params = attachment
        .pointer("/s3pending/params")
        .and_then(Value::as_object)
        .ok_or_else(|| PresignParseError::Shape("no s3pending.params object".to_owned()))?;
    let policy = params
        .get("policy")
        .and_then(Value::as_str)
        .ok_or_else(|| PresignParseError::Shape("no policy in params".to_owned()))?;
    let (bucket, starts_with) = policy_info(policy)?;

    let mut fields = Vec::new();
    for (name, value) in params {
        if let Some(text) = value.as_str() {
            fields.push((name.clone(), text.to_owned()));
        }
    }
    for field in starts_with {
        let value = match field.as_str() {
            "name" => file_name.to_owned(),
            "Content-Type" => content_type.to_owned(),
            "Content-Disposition" => format!("inline; filename=\"{file_name}\""),
            _ => String::new(),
        };
        fields.push((field, value));
    }
    Ok(PresignedUpload {
        s3_url: format!("https://{bucket}.s3.amazonaws.com/"),
        fields,
        attachment,
    })
}

/// The S3 form POST: the signed and starts-with fields, then the file last,
/// which is the ordering the policy grammar requires.
///
/// [`RequestAuth::Anonymous`], because the policy and its signature are two
/// of those form fields: the bucket authorises this request on its own, and
/// the seller's Tes session must not travel to Amazon with it.
#[must_use]
pub fn s3_upload_request(upload: &PresignedUpload, file: FilePart) -> HttpRequest {
    HttpRequest::post_multipart(
        upload.s3_url.clone(),
        upload.fields.clone(),
        Some(file),
        RequestAuth::Anonymous,
    )
}

/// The cover image's first hop. Tes's uploader hands cover images to
/// Uploadcare (its widget, public key below, observed 2026-09-19 on
/// `uploader/v2/{id}/files`): a multipart POST answering `{"file": uuid}`.
/// No credential of ours travels; the public key is the site's own and is
/// embedded in every uploader page.
pub const UPLOADCARE_UPLOAD_URL: &str = "https://upload.uploadcare.com/base/";
pub const UPLOADCARE_PUBLIC_KEY: &str = "5f93e59f109ebf8e3ed6";
/// The crop the widget is configured with (`crop: "700:525"`,
/// `imageShrink: "700x525"`), restated in the CDN modifiers Tes is told
/// about so the stored cover is the same shape the uploader would store.
pub const COVER_CDN_MODIFIERS: &str = "-/scale_crop/700x525/center/-/preview/";

#[must_use]
pub fn uploadcare_upload_request(file: FilePart) -> HttpRequest {
    HttpRequest::post_multipart(
        UPLOADCARE_UPLOAD_URL.to_owned(),
        vec![
            (
                "UPLOADCARE_PUB_KEY".to_owned(),
                UPLOADCARE_PUBLIC_KEY.to_owned(),
            ),
            ("UPLOADCARE_STORE".to_owned(), "auto".to_owned()),
        ],
        Some(file),
        RequestAuth::Anonymous,
    )
}

/// The second hop: Tes copies the Uploadcare file into its own bucket and
/// answers `{"fileName", "url"}`; `fileName` is the `customThumbnails` key
/// the draft metadata and the publish both carry. The body is the shape the
/// widget's `fileInfo` has, trimmed to what the route reads (verified
/// 2026-09-19: this exact body answered 200).
#[must_use]
pub fn upload_cover_image_request(
    uuid: &str,
    file_name: &str,
    content_type: &str,
    size: usize,
) -> HttpRequest {
    HttpRequest::post_json(
        format!("{ORIGIN}/teaching-resource/upload-cover-image"),
        json!({
            "uuid": uuid,
            "name": file_name,
            "size": size,
            "isStored": true,
            "isImage": true,
            "mimeType": content_type,
            "originalUrl": format!("https://ucarecdn.com/{uuid}/"),
            "cdnUrlModifiers": COVER_CDN_MODIFIERS,
            "cdnUrl": format!("https://ucarecdn.com/{uuid}/{COVER_CDN_MODIFIERS}"),
            "sourceInfo": { "source": "local", "file": {} },
        }),
    )
}

/// The cover key written onto the draft, the same route the metadata takes.
#[must_use]
pub fn set_cover_request(id: DraftId, key: &str) -> HttpRequest {
    HttpRequest::post_json(
        format!("{ORIGIN}/api/v2/resources/{}/draft", id.0),
        json!({ "customThumbnails": [{ "key": key }] }),
    )
}

/// The confirm handshake: the full presigned attachment object echoed back
/// with `type: file` and `isUploaded: true`, which is what makes the server
/// verify the S3 object by the key carried in `s3pending` (the M0 finding —
/// anything less leaves `isUploaded: false`).
#[must_use]
pub fn confirm_request(id: DraftId, upload: &PresignedUpload) -> HttpRequest {
    let mut echo = upload.attachment.clone();
    echo["type"] = json!("file");
    echo["isUploaded"] = json!(true);
    HttpRequest::post_json(
        format!("{ORIGIN}/api/resources/v3/draft/{}/attachment", id.0),
        json!([echo]),
    )
}

/// The publish that takes a draft live, and the one endpoint a paid listing's
/// price reaches: `POST .../{id}/draft/publish` re-posting the FULL metadata
/// beside the licence, with `price` when that licence is `TES-PAID`.
///
/// The route and the body are the browser's own, captured 2026-08-28 from a
/// draft-to-live publish. It supersedes the `POST .../{id}/publish` recorded
/// at M0 in docs/design/decisions.md, which no capture of this flow shows and
/// which carries no metadata for the licence to be validated against.
///
/// `primaryCategory` is the first category the draft declares, which is what
/// the capture carries, and it is omitted rather than invented when the draft
/// declares nothing to fill it with. `additionalAge` is not added here: it
/// rides the draft body itself, which is where the capture shows it.
#[must_use]
pub fn publish_request(id: DraftId, listing: &TesListing, thumbnails: Value) -> HttpRequest {
    let mut body = metadata_body(listing);
    // The draft's own `customThumbnails`, read back before the publish: the
    // publish re-posts the whole resource and an empty list here would drop
    // the cover the create set.
    body["customThumbnails"] = thumbnails;
    if let Some(primary) = listing.category_ids.first() {
        body["primaryCategory"] = json!(primary);
    }
    HttpRequest::post_json(
        format!("{ORIGIN}/api/v2/resources/{}/draft/publish", id.0),
        body,
    )
}

/// The draft overlay when it exists, else the published resource.
#[must_use]
pub fn read_draft_request(id: DraftId) -> HttpRequest {
    HttpRequest::get(format!("{ORIGIN}/api/v2/resources/{}/draft", id.0))
}

#[must_use]
pub fn read_resource_request(id: DraftId) -> HttpRequest {
    HttpRequest::get(format!("{ORIGIN}/api/v2/resources/{}", id.0))
}

/// The published resource's delete, answering 204 — the M0 reading, confirmed
/// against a live published resource by the 2026-08-28 capture. `DELETE
/// .../{id}/draft` removes only the draft overlay and answers 204 anyway (the
/// misleading-204 measured in M0), so which route a delete takes follows the
/// resource's state and the flow reports success only after the read for that
/// state returns 404.
#[must_use]
pub fn delete_resource_request(id: DraftId) -> HttpRequest {
    HttpRequest::delete(format!("{ORIGIN}/api/v2/resources/{}", id.0))
}

/// Deletes a never-published draft. `DELETE /resources/{id}` (the authoritative
/// published-resource delete) 404s for a draft-only resource without removing
/// it, so a draft is deleted through its own `/draft` route and the deletion is
/// verified by a `/draft` read, not a resource read that 404s either way.
#[must_use]
pub fn delete_draft_request(id: DraftId) -> HttpRequest {
    HttpRequest::delete(format!("{ORIGIN}/api/v2/resources/{}/draft", id.0))
}

/// How many rows one dashboard page asks for. The API paginates on `page`
/// and `limit` and answers an out-of-range page with an empty array.
pub const CATALOGUE_PAGE_LIMIT: u32 = 50;

/// One row of the seller's own catalogue as the dashboard list returns it.
/// `price_pence` is the API's own unit, carried without conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueEntry {
    pub id: i64,
    pub title: String,
    pub published: bool,
    pub licence: Option<String>,
    pub price_pence: Option<i64>,
}

impl CatalogueEntry {
    /// This row's durable identity in the spelling the write path records,
    /// which is what a reconcile has to bind: the import's own
    /// `teaching-resource/-{id}` form addresses the same resource and is a
    /// different string, and the ledger compares strings.
    #[must_use]
    pub fn remote(&self) -> RemoteListingId {
        DraftId(self.id).remote()
    }
}

#[must_use]
pub fn list_resources_request(page: u32, limit: u32) -> HttpRequest {
    HttpRequest::get(format!(
        "{ORIGIN}/api/v2/dashboard/getAllResources?page={page}&limit={limit}"
    ))
}

#[must_use]
pub fn list_drafts_request(page: u32, limit: u32) -> HttpRequest {
    HttpRequest::get(format!(
        "{ORIGIN}/api/v2/dashboard/getAllDrafts?page={page}&limit={limit}"
    ))
}

/// The seller's own tier record, and the only identity read the exclusivity
/// claim may issue.
///
/// The route names its principal as `me` and takes nothing else: no path
/// segment, no query, no body. There is therefore no parameter through which a
/// caller could point this at another seller's account, which is what makes
/// the `userId` it answers admissible as the value the global exclusivity lock
/// is taken on. See [`crate::identity`] for what is read off it.
#[must_use]
pub fn seller_tier_request() -> HttpRequest {
    HttpRequest::get(format!("{ORIGIN}/api/tier/gmv/me"))
}

/// The route that renews the seller's session cookies.
///
/// Tes rotates a session on this call: it answers with a fresh `Set-Cookie`
/// for the cookies whose values have moved on, which is how a captured jar is
/// kept alive rather than left to lapse a few hours after it was taken. It is
/// the same call `probes/session-longevity.sh` makes before each of its polls,
/// and that probe is the only method this repository has recorded for keeping
/// a captured session valid.
///
/// Like the tier read it names no account and takes nothing: there is no
/// parameter through which a caller could renew somebody else's session.
#[must_use]
pub fn refresh_cookies_request() -> HttpRequest {
    HttpRequest::get(format!("{ORIGIN}/api/authn/refresh-cookies"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CataloguePageError(pub String);

impl core::fmt::Display for CataloguePageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "catalogue page shape: {}", self.0)
    }
}

impl core::error::Error for CataloguePageError {}

/// Parses one dashboard page. `default_published` is what the endpoint the
/// page came from implies; a row's own `draft` flag overrides it where the
/// row carries one. A row without a numeric id fails the page rather than
/// being dropped, so a shape change cannot shorten a catalogue silently.
pub fn parse_catalogue_page(
    body: &Value,
    default_published: bool,
) -> Result<Vec<CatalogueEntry>, CataloguePageError> {
    let rows = body
        .as_array()
        .ok_or_else(|| CataloguePageError("the page is not a JSON array".to_owned()))?;
    rows.iter()
        .map(|row| {
            let id = row.get("id").and_then(Value::as_i64).ok_or_else(|| {
                CataloguePageError("a catalogue row carries no numeric id".to_owned())
            })?;
            Ok(CatalogueEntry {
                id,
                title: row
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                published: row
                    .get("draft")
                    .and_then(Value::as_bool)
                    .map_or(default_published, |draft| !draft),
                licence: row
                    .get("licence")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                price_pence: row.get("price").and_then(Value::as_i64),
            })
        })
        .collect()
}

/// Step one of the two-step own-file download: the manifest naming the
/// bundle. A draft has no bundle and this route answers it with an HTML
/// redirect to `?error=notfound` rather than JSON.
#[must_use]
pub fn download_manifest_request(id: DraftId) -> HttpRequest {
    HttpRequest::get(format!("{ORIGIN}/resource-detail/api/download/{}", id.0))
}

/// Step two: the bundle itself, at the path the manifest named. The upstream
/// answers a 302 to a signed CDN URL and the transport follows it, so what
/// comes back is the ZIP.
#[must_use]
pub fn download_bundle_request(path: &str) -> HttpRequest {
    HttpRequest::get(format!("{ORIGIN}{path}"))
}

/// Why a `Location` is not a destination this crate will re-issue to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectTargetError {
    /// Relative, or a scheme other than https.
    NotAbsoluteHttps,
    /// Carries userinfo, which is a credential in a url and a way to make one
    /// host's request look like another's.
    CarriesUserinfo,
    /// Resolves inside the machine or its network rather than out on the
    /// internet.
    NotPublic(String),
    /// The marketplace's own origin. The transport refuses this too, but
    /// there it is a `NotSent`, the class documented as the only one safe to
    /// retry, and a `Location` pointing back at Tes is permanent rather than
    /// a connectivity fault. Refusing it here names it instead.
    OurOwnOrigin,
}

impl core::fmt::Display for RedirectTargetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAbsoluteHttps => f.write_str("the location is not an absolute https url"),
            Self::CarriesUserinfo => f.write_str("the location carries userinfo"),
            Self::NotPublic(host) => {
                write!(f, "the location names {host}, which is not a public host")
            }
            Self::OurOwnOrigin => f.write_str(
                "the location names the marketplace's own origin, which a credential-free \
                 re-issue will not replay into",
            ),
        }
    }
}

impl core::error::Error for RedirectTargetError {}

/// Whether a `Location` a marketplace named is somewhere we will follow it.
///
/// The destination is chosen by the marketplace rather than by us, so this is
/// the only place its shape is examined before a request goes to it. The
/// private-network refusals are the reason this exists rather than the https
/// check: a `Location` is attacker-influenceable input the moment a
/// marketplace is compromised or simply wrong, and a fetch that follows one to
/// `127.0.0.1` or `169.254.169.254` is asking a machine on our side of the
/// network — or the seller's — to answer a request on the marketplace's
/// behalf. The device runs this on the seller's own machine, where the loopback
/// and link-local addresses are theirs.
pub fn check_redirect_target(url: &str) -> Result<(), RedirectTargetError> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(RedirectTargetError::NotAbsoluteHttps);
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err(RedirectTargetError::CarriesUserinfo);
    }
    let Some(host) = host_of_authority(authority) else {
        return Err(RedirectTargetError::NotAbsoluteHttps);
    };
    if host_of(ORIGIN).is_some_and(|ours| ours == host) {
        return Err(RedirectTargetError::OurOwnOrigin);
    }
    if is_private_host(&host) {
        return Err(RedirectTargetError::NotPublic(host));
    }
    Ok(())
}

/// The host of an absolute http(s) url, as the transport's routing reads one.
///
/// One parser, because this question is asked twice — here for a `Location`'s
/// address and in the transport for its not-our-origin test — and two
/// spellings of "host" answered it differently. Both halves of that mattered:
/// a byte comparison is not a host comparison, and the session arm failed
/// closed on `WWW.TES.COM` while the not-our-origin arm failed open on it,
/// which is a route back into the session origin chosen by a `Location`
/// rather than by us.
///
/// `None` when the url names no host this code can read, which every caller
/// turns into a refusal: a request whose destination cannot be named is not
/// one to send.
pub(crate) fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    host_of_authority(rest.split(['/', '?', '#']).next()?)
}

/// Userinfo stripped, port stripped, trailing dot stripped, lowercased, and
/// an IPv6 literal handed back without its brackets so it parses as the
/// address it is.
///
/// An IPv6 literal is bracketed and full of colons, so the port cannot be
/// split off by the first one. Getting this wrong reads `[::1]` as a host
/// named `[`, which parses as no address at all and therefore passes every
/// private-address test — the loopback case sails through precisely because
/// it is the awkward one to parse.
fn host_of_authority(authority: &str) -> Option<String> {
    let after_userinfo = authority.rsplit('@').next()?;
    let host = match after_userinfo.strip_prefix('[') {
        Some(literal) => literal.split_once(']')?.0,
        None => after_userinfo.split(':').next()?,
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Hosts a redirect may not name.
///
/// Literal addresses only, deliberately. A name that resolves to a private
/// address defeats this and no string test can close that — the closing move
/// is at connection time and this crate opens no sockets. What it does stop is
/// the direct form, which is the one a `Location` actually carries, and it
/// stops it without a DNS lookup that would itself be a request to something
/// the marketplace named.
fn is_private_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    let Ok(address) = host.parse::<std::net::IpAddr>() else {
        return false;
    };
    match address {
        std::net::IpAddr::V4(v4) => is_private_v4(v4),
        // `fe80::/10` link-local, `fc00::/7` unique-local — the range every
        // home and corporate network actually numbers itself out of — and
        // then the same address written as IPv6: the kernel routes an
        // IPv4-mapped destination to the IPv4 address it names, so
        // `::ffff:127.0.0.1` reaches the loopback that `127.0.0.1` does and
        // the IPv4 predicate is the one that decides it.
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.segments()[0] & 0xffc0 == 0xfe80
                || v6.segments()[0] & 0xfe00 == 0xfc00
                || v6.to_ipv4_mapped().is_some_and(is_private_v4)
        }
    }
}

fn is_private_v4(v4: std::net::Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.octets()[0] == 0
}

/// The hop the bundle route named, re-issued carrying nothing of ours.
///
/// The url is the marketplace's own `Location` rather than anything composed
/// here, which is exactly why it travels as [`RequestAuth::Redirected`]: the
/// transport's rule for it is about the request's shape — https, not our
/// origin, bounded body — because no host constant can be maintained for a
/// content network the marketplace may re-point.
#[must_use]
pub fn redirected_bundle_request(url: String) -> HttpRequest {
    HttpRequest {
        method: Method::Get,
        url,
        body: RequestBody::Empty,
        auth: RequestAuth::Redirected,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadManifestError {
    /// The expected manifest object or a named bundle's URL is malformed.
    InvalidShape,
    /// No bundle for this resource — the shape a draft produces.
    NoPublishedBundle,
    /// The manifest named somewhere other than a path on this origin.
    OffOrigin(String),
}

impl core::fmt::Display for DownloadManifestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidShape => write!(f, "the download manifest has an invalid zipUrls entry"),
            Self::NoPublishedBundle => write!(f, "no published bundle for this resource"),
            Self::OffOrigin(url) => write!(f, "the manifest named an off-origin url: {url}"),
        }
    }
}

impl core::error::Error for DownloadManifestError {}

/// Recovers the bundle path from a download manifest, which keys `zipUrls`
/// by the resource id as a string.
///
/// The returned path must be origin-relative, and the reason has changed
/// rather than expired. It was written for the broker's leased gateway, where
/// an absolute url would skip the transport's rebasing and escape the
/// allow-list; with the download on the seller's own device there is no
/// gateway to escape. What the rule does now is keep the first hop on the
/// session host, so the one request that carries the seller's cookie is one
/// this crate composed rather than one the marketplace named. Where the
/// bundle really lives is then the redirect's business, and the redirect is
/// followed — if at all — by a client carrying nothing.
pub fn parse_download_manifest(body: &Value, id: DraftId) -> Result<String, DownloadManifestError> {
    let urls = body
        .get("zipUrls")
        .and_then(Value::as_object)
        .ok_or(DownloadManifestError::InvalidShape)?;
    let entry = urls
        .get(&id.0.to_string())
        .ok_or(DownloadManifestError::NoPublishedBundle)?;
    let url = entry
        .get("url")
        .and_then(Value::as_str)
        .ok_or(DownloadManifestError::InvalidShape)?;
    if url.starts_with('/') && !url.starts_with("//") {
        Ok(url.to_owned())
    } else {
        Err(DownloadManifestError::OffOrigin(url.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_catalogue_page, parse_download_manifest, parse_presign, CataloguePageError,
        DownloadManifestError, DraftId, FreeLicence, NotAPrice, PresignParseError, ReadLicence,
        TesAges, TesLicence, TesListing, TesPrice, TesPricing,
    };

    #[test]
    fn a_redirect_target_is_public_https_and_carries_no_credential() {
        use super::{check_redirect_target, RedirectTargetError};
        assert_eq!(
            check_redirect_target("https://d111111abcdef8.cloudfront.net/b?Signature=a"),
            Ok(())
        );
        for bad in ["/relative/path", "http://cdn.example/bundle", "https://"] {
            assert_eq!(
                check_redirect_target(bad),
                Err(RedirectTargetError::NotAbsoluteHttps),
                "{bad} is not an absolute https destination"
            );
        }
        // Our own origin, however it is spelled. The trailing-dot form is the
        // one that reaches here live: reqwest's policy calls it a host change
        // and hands the hop back, and it resolves to the session the flow is
        // already holding. Refused by name rather than left to the transport,
        // where it is a `NotSent` and therefore reported as safe to retry.
        for ours in [
            "https://www.tes.com/api/v2/resources/1",
            "https://WWW.TES.COM/api/v2/resources/1",
            "https://www.tes.com./api/v2/resources/1",
        ] {
            assert_eq!(
                check_redirect_target(ours),
                Err(RedirectTargetError::OurOwnOrigin),
                "{ours} is the marketplace's own origin however it is spelled"
            );
        }
        assert_eq!(
            check_redirect_target("https://user:pass@cdn.example/bundle"),
            Err(RedirectTargetError::CarriesUserinfo),
            "a credential in a url is a credential, and it also makes one host's request \
             look like another's"
        );
        // The reason this function exists. Following a marketplace-named
        // location to any of these asks a machine on the seller's own side of
        // the network to answer a request the marketplace chose.
        for inside in [
            "https://127.0.0.1/bundle",
            "https://localhost/bundle",
            "https://10.0.0.5/bundle",
            "https://192.168.1.1/bundle",
            "https://169.254.169.254/latest/meta-data/",
            "https://[::1]/bundle",
            // The same machines addressed in IPv6. An IPv4-mapped address is
            // routed to the IPv4 address it names, so a predicate that reads
            // these as unfamiliar IPv6 refuses none of them, and unique-local
            // is the range a home or corporate network actually uses.
            "https://[::ffff:127.0.0.1]/bundle",
            "https://[0:0:0:0:0:ffff:7f00:1]/bundle",
            "https://[::ffff:10.0.0.5]/bundle",
            "https://[::ffff:169.254.169.254]/latest/meta-data/",
            "https://[fc00::1]/bundle",
            "https://[fd00::1]/bundle",
        ] {
            assert!(
                matches!(
                    check_redirect_target(inside),
                    Err(RedirectTargetError::NotPublic(_))
                ),
                "{inside} is inside the machine or its network and must be refused"
            );
        }
    }

    use base64::Engine;
    use tam_types::CopyFormat;

    /// The read side is a transcription of a polled vocabulary, so it is
    /// checked against that vocabulary rather than trusted. The paid/free
    /// semantics live in the JSON and in a prose comment and in no
    /// consultable type, which is exactly the condition under which a
    /// transcription drifts.
    /// The same rule for the age bands: the table drives what the wire's
    /// `ages` array holds, so a transcription slip there posts ages the
    /// seller never declared.
    #[test]
    fn the_age_bands_agree_with_the_polled_vocabulary() {
        let vocabulary: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/design/data/tes-vocabulary.json"
        ))
        .expect("the polled vocabulary parses");
        let options = vocabulary["ageRanges"]["options"]
            .as_object()
            .expect("ageRanges.options is an object");
        assert_eq!(
            options.len(),
            super::AGE_BANDS.len(),
            "the picker offers seven bands and the table models all seven"
        );
        for band in &super::AGE_BANDS {
            let row = options
                .get(&band.id.to_string())
                .unwrap_or_else(|| panic!("band {} is a polled row", band.id));
            let polled: Vec<i64> = row["humanAges"]
                .as_array()
                .expect("every band publishes its humanAges")
                .iter()
                .filter_map(serde_json::Value::as_i64)
                .collect();
            assert_eq!(
                polled, band.human_ages,
                "band {} posts the ages Tes publishes for it",
                band.id
            );
        }
    }

    /// The union rule, on the capture's own disjoint pair. A contiguous fill
    /// would post everything from 5 to 18 here.
    #[test]
    fn a_disjoint_declaration_posts_the_union_of_its_bands_and_not_the_fill() {
        let derived = TesAges::new(vec![2, 6])
            .derived_ages()
            .expect("both are bands");
        assert_eq!(
            derived,
            super::DerivedAges {
                ages: vec![5, 6, 7, 16, 17, 18],
                main_age: Some(2),
            },
            "the 2026-08-29 capture posts exactly this array for bands 2 and 6"
        );
    }

    /// P.1, settled. `16+` closes at 18 on the wire, so a 16+-only listing
    /// states real ages rather than omitting the pair.
    #[test]
    fn the_sixteen_plus_band_alone_posts_its_own_closed_age_set() {
        let derived = TesAges::new(vec![6]).derived_ages().expect("6 is a band");
        assert_eq!(
            derived,
            super::DerivedAges {
                ages: vec![16, 17, 18],
                main_age: Some(6),
            },
            "the uploader bootstrap publishes humanAges [16,17,18] for band 6"
        );
    }

    /// And the one band that really has no ages still omits both keys, which
    /// is what the not-applicable row means.
    #[test]
    fn the_not_applicable_band_states_neither_key() {
        for declaration in [TesAges::new(vec![7]), TesAges::new(vec![])] {
            let derived = declaration.derived_ages().expect("7 is a band");
            assert_eq!(
                derived,
                super::DerivedAges {
                    ages: vec![],
                    main_age: None,
                },
                "an empty age set states no ages and no main band"
            );
        }
    }

    #[test]
    fn a_grade_id_outside_the_band_vocabulary_is_named_rather_than_dropped() {
        assert_eq!(
            TesAges::new(vec![4, 99]).derived_ages(),
            Err(super::UnknownAgeBand(99)),
            "the ids are seeded from this vocabulary, so one that is not in it is a fault"
        );
    }

    /// The draft POST carries `additionalAge`: entry 44 of the 2026-08-29
    /// capture is a draft and holds it beside `mainAge`.
    #[test]
    fn the_draft_body_carries_the_additional_band_beside_the_main_one() {
        let listing = TesListing {
            age_channel: TesAges::new(vec![2, 6]),
            ages: vec![5, 6, 7, 16, 17, 18],
            main_age: Some(6),
            ..listing(TesPricing::Free(FreeLicence::CcBy))
        };
        let RequestBody::Json(body) = super::set_metadata_request(DraftId(1), &listing).body else {
            panic!("the draft metadata is a JSON post");
        };
        assert_eq!(body["mainAge"], serde_json::json!(6));
        assert_eq!(body["additionalAge"], serde_json::json!(2));
        assert_eq!(
            body["ages"],
            serde_json::json!([5, 6, 7, 16, 17, 18]),
            "the draft carries the whole derivation, not a fragment of it"
        );
    }

    #[test]
    fn the_seven_tokens_agree_with_the_polled_vocabulary() {
        let vocabulary: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/design/data/tes-vocabulary.json"
        ))
        .expect("the polled vocabulary parses");
        let options = vocabulary["licences"]["options"]
            .as_object()
            .expect("licences.options is an object");
        assert_eq!(
            options.len(),
            ReadLicence::ALL.len(),
            "RefdataStore holds seven rows and the adapter models all seven; the four-value \
             record refused three real licences as unrecognised"
        );
        for licence in ReadLicence::ALL {
            let row = options
                .get(licence.as_str())
                .unwrap_or_else(|| panic!("{} is a polled row", licence.as_str()));
            assert_eq!(
                row["paid"].as_bool(),
                Some(licence.is_paid()),
                "{} classifies as the poll recorded it, and the price gate reads off this",
                licence.as_str()
            );
        }
    }

    #[test]
    fn an_unlisted_token_is_unrecognised_rather_than_free() {
        assert_eq!(
            ReadLicence::from_token("CC-BY-NC"),
            None,
            "reading an unknown licence as free would state a rights grant nobody made"
        );
        assert_eq!(
            ReadLicence::from_token("TES-PAID-SCHOOL"),
            Some(ReadLicence::TesPaidSchool),
            "the school tier is real on read even though this adapter never writes it"
        );
    }

    #[test]
    fn the_writable_four_are_a_subset_of_the_seven_the_store_holds() {
        for writable in [
            TesLicence::CcBy,
            TesLicence::CcBySa,
            TesLicence::CcByNd,
            TesLicence::TesPaid,
        ] {
            assert!(
                ReadLicence::from_token(writable.as_str()).is_some(),
                "{} is written by this adapter and must read back",
                writable.as_str()
            );
        }
        assert_eq!(
            ReadLicence::ALL
                .into_iter()
                .filter(|licence| licence.is_paid())
                .map(ReadLicence::as_str)
                .collect::<Vec<_>>(),
            vec!["TES-PAID", "TES-PAID-SCHOOL"],
            "the two paid tiers are the ones that gate the write on a price"
        );
    }

    use serde_json::json;
    use tam_marketplace::transport::RequestBody;

    #[test]
    fn the_licence_strings_match_the_api_enum() {
        for (licence, expected) in [
            (TesLicence::CcBy, "CC-BY"),
            (TesLicence::CcBySa, "CC-BY-SA"),
            (TesLicence::CcByNd, "CC-BY-ND"),
            (TesLicence::TesPaid, "TES-PAID"),
        ] {
            assert_eq!(
                licence.as_str(),
                expected,
                "the API validates these exact strings"
            );
        }
    }

    fn listing(pricing: TesPricing) -> TesListing {
        TesListing {
            title: "T".to_owned(),
            description_raw: "D".to_owned(),
            description_format: CopyFormat::Markdown,
            category_ids: vec![1_000_448, 1_000_977],
            age_channel: TesAges::new(vec![3, 4]),
            ages: vec![11, 12],
            main_type: Some(99_009),
            main_age: Some(4),
            pricing,
        }
    }

    fn paid(minor_units: i64) -> TesPricing {
        TesPricing::Paid(TesPrice::new(minor_units).expect("the fixture price is positive"))
    }

    #[test]
    fn a_price_is_a_positive_number_of_minor_units() {
        assert_eq!(
            TesPrice::new(500).map(TesPrice::minor_units),
            Ok(500),
            "the captured publish body carries GBP 5.00 as the integer 500"
        );
        for refused in [0, -1] {
            assert_eq!(
                TesPrice::new(refused),
                Err(NotAPrice(refused)),
                "a non-positive amount is not a price; free is a Creative Commons licence"
            );
        }
    }

    #[test]
    fn the_pricing_pairs_each_licence_with_what_the_api_demands_of_it() {
        assert_eq!(
            TesPricing::Free(FreeLicence::CcBySa).licence(),
            TesLicence::CcBySa,
            "a free listing carries the Creative Commons licence it named"
        );
        assert_eq!(
            TesPricing::Free(FreeLicence::CcBySa).price(),
            None,
            "the API refuses a price against a Creative Commons licence"
        );
        assert_eq!(
            paid(500).licence(),
            TesLicence::TesPaid,
            "a priced listing is TES-PAID and nothing else"
        );
        assert_eq!(
            paid(500).price().map(TesPrice::minor_units),
            Some(500),
            "and it cannot exist without the price the API refuses it without"
        );
    }

    #[test]
    fn metadata_request_carries_the_licence_and_the_markdown_type() {
        let listing = listing(TesPricing::Free(FreeLicence::CcBy));
        let request = super::set_metadata_request(DraftId(7), &listing);
        let RequestBody::Json(body) = &request.body else {
            panic!("metadata is a JSON body");
        };
        assert_eq!(
            body["licence"], "CC-BY",
            "publish is refused without a valid licence on the draft"
        );
        assert_eq!(
            body["descriptionRawType"], "md",
            "the description type must accompany the raw markdown"
        );
        assert_eq!(
            body["categories"][0]["id"], 1_000_448,
            "categories travel as objects carrying ids"
        );
        assert_eq!(
            body.get("price"),
            None,
            "a free draft names no price, which is what the licence demands of it"
        );
    }

    #[test]
    fn a_paid_draft_carries_the_price_the_tes_paid_licence_is_refused_without() {
        let request = super::set_metadata_request(DraftId(7), &listing(paid(500)));
        let RequestBody::Json(body) = &request.body else {
            panic!("metadata is a JSON body");
        };
        assert_eq!(
            body["licence"], "TES-PAID",
            "the paid licence is the one the capture pairs with a price"
        );
        assert_eq!(
            body["price"], 500,
            "the price rides in minor units, the integer the wire carries"
        );
    }

    #[test]
    fn the_publish_body_restates_the_metadata_with_the_price_and_the_primary_category() {
        let request = super::publish_request(DraftId(13_264_370), &listing(paid(500)), json!([]));
        assert_eq!(
            request.url, "https://www.tes.com/api/v2/resources/13264370/draft/publish",
            "the captured publish is the draft's own route, not the resource's"
        );
        let RequestBody::Json(body) = &request.body else {
            panic!("publish is a JSON body");
        };
        assert_eq!(
            body["licence"], "TES-PAID",
            "publish is what carries the paid licence live"
        );
        assert_eq!(body["price"], 500, "and the price it is refused without");
        assert_eq!(
            body["primaryCategory"], 1_000_448,
            "the primary category is the first the draft declares"
        );
        assert_eq!(
            body["additionalAge"], 3,
            "the additional age is the range beside mainAge, as the capture carries it"
        );
        assert_eq!(
            body["mainAge"], 4,
            "publish restates the whole draft rather than referring to it"
        );
        assert_eq!(
            body["descriptionRawType"], "md",
            "the description type accompanies the raw markdown here too"
        );
        assert_eq!(
            body["customThumbnails"],
            serde_json::json!([]),
            "the capture carries the empty thumbnail list"
        );
        assert_eq!(
            body["categories"][1]["id"], 1_000_977,
            "every category travels, not only the primary one"
        );
    }

    #[test]
    fn a_free_publish_carries_a_creative_commons_licence_and_no_price() {
        let request = super::publish_request(
            DraftId(7),
            &TesListing {
                category_ids: Vec::new(),
                age_channel: TesAges::new(vec![4]),
                ..listing(TesPricing::Free(FreeLicence::CcByNd))
            },
            json!([]),
        );
        let RequestBody::Json(body) = &request.body else {
            panic!("publish is a JSON body");
        };
        assert_eq!(
            body["licence"], "CC-BY-ND",
            "a free publish is licensed, not priced"
        );
        assert_eq!(
            body.get("price"),
            None,
            "no price is invented for a listing that has none"
        );
        assert_eq!(
            body.get("primaryCategory"),
            None,
            "a draft declaring no category names no primary one rather than a guessed id"
        );
        assert_eq!(
            body.get("additionalAge"),
            None,
            "and a draft with only its main age range carries no additional one"
        );
    }

    #[test]
    fn presign_parsing_recovers_bucket_fields_and_echo() {
        let policy = base64::engine::general_purpose::STANDARD.encode(
            json!({
                "conditions": [
                    {"bucket": "tes-uploads"},
                    ["starts-with", "$name", ""],
                    ["starts-with", "$Content-Type", ""],
                ]
            })
            .to_string(),
        );
        let body = json!([{
            "tempId": "TEMP-0",
            "s3pending": {"key": "k/1", "params": {"policy": policy, "signature": "sig"}},
        }]);
        let upload =
            parse_presign(&body, "pack.pdf", "application/pdf").expect("the presign parses");
        assert_eq!(
            upload.s3_url, "https://tes-uploads.s3.amazonaws.com/",
            "the bucket comes from the decoded policy, nowhere else"
        );
        assert!(
            upload
                .fields
                .iter()
                .any(|(name, value)| name == "name" && value == "pack.pdf"),
            "starts-with fields are filled from the file"
        );
        assert!(
            upload.fields.iter().any(|(name, _)| name == "signature"),
            "signed params pass through as form fields"
        );
        let confirm = super::confirm_request(DraftId(7), &upload);
        let RequestBody::Json(echo) = &confirm.body else {
            panic!("confirm is a JSON body");
        };
        assert_eq!(
            echo[0]["isUploaded"], true,
            "the confirm echo asserts the upload, which is what flips the server state"
        );
        assert_eq!(
            echo[0]["s3pending"]["key"], "k/1",
            "the echo carries the S3 key the server verifies by"
        );
    }

    #[test]
    fn a_policy_without_a_bucket_is_refused() {
        let policy =
            base64::engine::general_purpose::STANDARD.encode(json!({"conditions": []}).to_string());
        let body = json!([{"s3pending": {"params": {"policy": policy}}}]);
        assert!(
            matches!(
                parse_presign(&body, "f", "application/pdf"),
                Err(PresignParseError::Policy(_))
            ),
            "an upload with no destination bucket must fail closed"
        );
    }

    #[test]
    fn a_catalogue_row_without_an_id_fails_the_page() {
        let page = json!([{"title": "no id here"}]);
        assert!(
            matches!(
                parse_catalogue_page(&page, true),
                Err(CataloguePageError(_))
            ),
            "a row the walk cannot address must fail the page, not vanish from it"
        );
    }

    #[test]
    fn a_rows_own_draft_flag_overrides_the_endpoint_default() {
        let page = json!([{"id": 1, "draft": true}, {"id": 2}]);
        let entries = parse_catalogue_page(&page, true).expect("the page parses");
        assert!(
            !entries[0].published,
            "a row declaring itself a draft is not published, whatever list it came from"
        );
        assert!(
            entries[1].published,
            "a row with no draft flag takes the endpoint's meaning"
        );
    }

    #[test]
    fn the_manifest_yields_the_bundle_path_keyed_by_the_resource_id() {
        let manifest = json!({
            "zipUrls": {
                "9001": {"url": "/teaching-resource/download/9001/bundle", "title": "Pack"}
            }
        });
        assert_eq!(
            parse_download_manifest(&manifest, DraftId(9001)),
            Ok("/teaching-resource/download/9001/bundle".to_owned()),
            "the bundle path comes from the manifest, not from a path we assumed"
        );
    }

    #[test]
    fn an_empty_bundle_map_reports_no_published_bundle() {
        assert_eq!(
            parse_download_manifest(&json!({"zipUrls": {}}), DraftId(9001)),
            Err(DownloadManifestError::NoPublishedBundle),
            "a draft has no bundle, which is a distinct answer from a failed read"
        );
    }

    #[test]
    fn a_manifest_naming_another_origin_is_refused() {
        for url in ["https://elsewhere.test/steal", "//elsewhere.test/steal"] {
            let manifest = json!({"zipUrls": {"9001": {"url": url}}});
            assert!(
                matches!(
                    parse_download_manifest(&manifest, DraftId(9001)),
                    Err(DownloadManifestError::OffOrigin(_))
                ),
                "an absolute url would bypass the gateway allow-list entirely: {url}"
            );
        }
    }
}
