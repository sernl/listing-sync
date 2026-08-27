//! Typed builders and parsers for the M0-confirmed endpoints, recorded in
//! docs/design/decisions.md: create, metadata, presign-and-confirm, the S3
//! form POST, publish (which requires a licence on the draft first), read and
//! delete. Builders return seam `HttpRequest` values so cassettes and the
//! live transport are interchangeable.

use base64::Engine;
use serde_json::{json, Value};
use tam_marketplace::transport::{FilePart, HttpRequest, Method, RequestBody};

pub const ORIGIN: &str = "https://www.tes.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DraftId(pub i64);

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

/// The metadata the draft endpoint accepts, in the field names the API uses.
/// Category and age identifiers arrive already projected; the taxonomy hub
/// (M1g) owns how they are derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TesListing {
    pub title: String,
    pub description_markdown: String,
    pub category_ids: Vec<i64>,
    pub age_range_ids: Vec<i64>,
    pub ages: Vec<i64>,
    pub main_type: i64,
    pub main_age: i64,
    pub licence: TesLicence,
}

#[must_use]
pub fn create_draft_request() -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        url: format!("{ORIGIN}/api/v2/resources"),
        body: RequestBody::Json(json!({})),
    }
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
        description_markdown: "Automated schema probe. **Delete me.**".to_owned(),
        category_ids: vec![1_000_448],
        age_range_ids: vec![4],
        ages: vec![11, 12, 13, 14],
        main_type: 99_009,
        main_age: 4,
        licence: TesLicence::CcBy,
    }
}

#[must_use]
pub fn set_metadata_request(id: DraftId, listing: &TesListing) -> HttpRequest {
    let categories: Vec<Value> = listing
        .category_ids
        .iter()
        .map(|category| json!({ "id": category }))
        .collect();
    HttpRequest {
        method: Method::Post,
        url: format!("{ORIGIN}/api/v2/resources/{}/draft", id.0),
        body: RequestBody::Json(json!({
            "title": listing.title,
            "descriptionRaw": listing.description_markdown,
            "descriptionRawType": "md",
            "categories": categories,
            "ageRanges": listing.age_range_ids,
            "ages": listing.ages,
            "yearGroups": [],
            "mainType": listing.main_type,
            "mainAge": listing.main_age,
            "licence": listing.licence.as_str(),
        })),
    }
}

#[must_use]
pub fn presign_request(id: DraftId, file_name: &str, temp_id: &str) -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        url: format!("{ORIGIN}/api/resources/v3/draft/{}/attachment", id.0),
        body: RequestBody::Json(json!([{
            "name": file_name,
            "tempId": temp_id,
            "previewOption": 0,
        }])),
    }
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
#[must_use]
pub fn s3_upload_request(upload: &PresignedUpload, file: FilePart) -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        url: upload.s3_url.clone(),
        body: RequestBody::Multipart {
            fields: upload.fields.clone(),
            file: Some(file),
        },
    }
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
    HttpRequest {
        method: Method::Post,
        url: format!("{ORIGIN}/api/resources/v3/draft/{}/attachment", id.0),
        body: RequestBody::Json(json!([echo])),
    }
}

#[must_use]
pub fn publish_request(id: DraftId) -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        url: format!("{ORIGIN}/api/v2/resources/{}/publish", id.0),
        body: RequestBody::Json(json!({})),
    }
}

/// The draft overlay when it exists, else the published resource.
#[must_use]
pub fn read_draft_request(id: DraftId) -> HttpRequest {
    HttpRequest {
        method: Method::Get,
        url: format!("{ORIGIN}/api/v2/resources/{}/draft", id.0),
        body: RequestBody::Empty,
    }
}

#[must_use]
pub fn read_resource_request(id: DraftId) -> HttpRequest {
    HttpRequest {
        method: Method::Get,
        url: format!("{ORIGIN}/api/v2/resources/{}", id.0),
        body: RequestBody::Empty,
    }
}

/// The REAL delete. `DELETE .../{id}/draft` removes only the draft overlay
/// and answers 204 anyway — the misleading-204 measured in M0 — so this
/// module deliberately offers no draft-delete builder, and the delete flow
/// reports success only after the API read returns 404.
#[must_use]
pub fn delete_resource_request(id: DraftId) -> HttpRequest {
    HttpRequest {
        method: Method::Delete,
        url: format!("{ORIGIN}/api/v2/resources/{}", id.0),
        body: RequestBody::Empty,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_presign, DraftId, PresignParseError, TesLicence, TesListing};
    use base64::Engine;
    use serde_json::json;

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

    #[test]
    fn metadata_request_carries_the_licence_and_the_markdown_type() {
        let listing = TesListing {
            title: "T".to_owned(),
            description_markdown: "D".to_owned(),
            category_ids: vec![1_000_448],
            age_range_ids: vec![4],
            ages: vec![11, 12],
            main_type: 99_009,
            main_age: 4,
            licence: TesLicence::CcBy,
        };
        let request = super::set_metadata_request(DraftId(7), &listing);
        let super::RequestBody::Json(body) = &request.body else {
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
        let super::RequestBody::Json(echo) = &confirm.body else {
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
}
