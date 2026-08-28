//! The S3 half of the upload: AWS Signature Version 2 built against a typed
//! upload ticket, the multipart loop, and the two XML shapes that bracket it.
//!
//! TPT holds the AWS secret and exposes a signing oracle — hand it a
//! `StringToSign`, get back the base64 HMAC. The oracle signs whatever it is
//! given, so the canonicalised resource is the only thing standing between a
//! correct upload and a signature over somebody else's object. That is why
//! [`StringToSign`] has no public constructor taking a string: it is built
//! from an [`UploadTicket`] and the operation, and [`sign_auth_scope`] proves
//! the resource still names that ticket before the request is built.

use base64::Engine as _;
use md5::{Digest as _, Md5};

/// Path-style, not virtual-host: every captured S3 call addresses
/// `https://s3.amazonaws.com/<bucket>/<key>`, and the canonicalised resource
/// the signature covers has the same shape.
pub const S3_ENDPOINT: &str = "https://s3.amazonaws.com";

/// The part size the multipart loop cuts at. No capture exercises more than
/// one part — the observed upload was 223 791 bytes in a single part — so
/// this is Evaporate.js's own default rather than a measured TPT threshold,
/// and it is also S3's documented minimum for a non-final part. Both agree at
/// five mebibytes, which is why a value nothing has measured is still safe.
pub const PART_SIZE: usize = 5 * 1024 * 1024;

/// The upload slot a ticket was reserved for. Only `product` is captured; the
/// preview, video and manual-thumbnail slots are named by the form's config
/// and have never been exercised, so the write path uses `product` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadSlot {
    Product,
}

impl UploadSlot {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Product => "product",
        }
    }
}

/// The bucket and object key `/uploads/upload_file` chose. Both are the
/// server's, never ours: the key embeds a shard, the seller id and the UTC
/// date, and computing one locally would put our bytes at an address TPT does
/// not expect.
///
/// `Debug` redacts the path, which carries the seller id.
#[derive(Clone, PartialEq, Eq)]
pub struct UploadTicket {
    bucket: String,
    path: String,
}

impl core::fmt::Debug for UploadTicket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "UploadTicket(bucket {}, path redacted)", self.bucket)
    }
}

impl UploadTicket {
    #[must_use]
    pub const fn new(bucket: String, path: String) -> Self {
        Self { bucket, path }
    }

    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The object's absolute url under path-style addressing, with the
    /// operation's query string appended.
    #[must_use]
    pub fn object_url(&self, query: &str) -> String {
        let base = format!("{S3_ENDPOINT}/{}/{}", self.bucket, self.path);
        if query.is_empty() {
            base
        } else {
            format!("{base}?{query}")
        }
    }

    /// The canonicalised resource AWS SigV2 signs: `/<bucket>/<key>` plus the
    /// subresource query. Built here and nowhere else, so no caller can hand
    /// the oracle a resource this ticket does not name.
    fn canonical_resource(&self, query: &str) -> String {
        let base = format!("/{}/{}", self.bucket, self.path);
        if query.is_empty() {
            base
        } else {
            format!("{base}?{query}")
        }
    }

    /// The prefix every `StringToSign` built from this ticket must carry.
    fn resource_prefix(&self) -> String {
        format!("/{}/{}", self.bucket, self.path)
    }
}

/// The three S3 calls the upload makes, each carrying the query string that
/// is part of the signed resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S3Operation {
    Initiate,
    UploadPart { part_number: u32, upload_id: String },
    Complete { upload_id: String },
}

impl S3Operation {
    #[must_use]
    pub fn verb(&self) -> &'static str {
        match *self {
            Self::Initiate | Self::Complete { .. } => "POST",
            Self::UploadPart { .. } => "PUT",
        }
    }

    #[must_use]
    pub fn query(&self) -> String {
        match *self {
            Self::Initiate => "uploads".to_owned(),
            Self::UploadPart {
                part_number,
                ref upload_id,
            } => format!("partNumber={part_number}&uploadId={upload_id}"),
            Self::Complete { ref upload_id } => format!("uploadId={upload_id}"),
        }
    }
}

/// An AWS SigV2 `StringToSign`, constructible only from an [`UploadTicket`].
/// Holding one is holding the statement that a signature will cover exactly
/// this verb, digest, content type, date and object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringToSign {
    text: String,
    resource: String,
}

impl StringToSign {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn resource(&self) -> &str {
        &self.resource
    }
}

/// The signature this crate cannot produce and must ask TPT for. `Debug`
/// redacts: it authorises one write against a live AWS key.
#[derive(Clone, PartialEq, Eq)]
pub struct S3Signature(String);

impl core::fmt::Debug for S3Signature {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("S3Signature(redacted)")
    }
}

impl S3Signature {
    #[must_use]
    pub const fn new(signature: String) -> Self {
        Self(signature)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The published AWS access key id the form page bootstraps, and nothing else
/// about the account: the secret never leaves TPT. Not redacted, because a
/// key id without its secret authorises nothing — but it is a live upstream
/// identifier, so no fixture carries the real one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsKeyId(String);

impl AwsKeyId {
    #[must_use]
    pub const fn new(key_id: String) -> Self {
        Self(key_id)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Builds the `StringToSign` for one operation over one ticket. The `Date`
/// slot is deliberately empty: the request carries `x-amz-date` instead, and
/// SigV2 requires exactly one of the two.
#[must_use]
pub fn string_to_sign(
    ticket: &UploadTicket,
    operation: &S3Operation,
    content_md5: Option<&str>,
    content_type: &str,
    amz_date: &str,
) -> StringToSign {
    let resource = ticket.canonical_resource(&operation.query());
    let digest = content_md5.unwrap_or_default();
    let text = format!(
        "{verb}\n{digest}\n{content_type}\n\nx-amz-date:{amz_date}\n{resource}",
        verb = operation.verb(),
    );
    StringToSign { text, resource }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S3Error {
    /// The string about to be signed names an object this ticket does not.
    /// The oracle would sign it regardless, which is precisely why this is
    /// checked here and not there.
    ResourceOutOfScope { expected: String, found: String },
    /// An XML answer that does not carry the element the next hop needs.
    Shape(String),
    /// A file whose part count cannot be represented, or an empty upload.
    Unplannable(String),
}

impl core::fmt::Display for S3Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ResourceOutOfScope { expected, found } => write!(
                f,
                "the signing oracle was asked for a resource outside this upload: expected a \
                 string over {expected}, found one over {found}"
            ),
            Self::Shape(detail) => write!(f, "S3 answered an unexpected shape: {detail}"),
            Self::Unplannable(detail) => write!(f, "the upload cannot be planned: {detail}"),
        }
    }
}

impl core::error::Error for S3Error {}

/// The scoping guard. `sign_auth` will HMAC any string an authenticated
/// session hands it, with no observable check that the canonicalised resource
/// belongs to the caller, so this crate does the check the upstream does not:
/// a string whose resource does not name this ticket's object never reaches
/// the oracle.
pub fn sign_auth_scope(ticket: &UploadTicket, to_sign: &StringToSign) -> Result<(), S3Error> {
    let prefix = ticket.resource_prefix();
    let scoped = to_sign
        .resource()
        .get(..prefix.len())
        .is_some_and(|head| head == prefix)
        && to_sign
            .resource()
            .get(prefix.len()..)
            .is_some_and(|tail| tail.is_empty() || tail.starts_with('?'));
    if scoped {
        Ok(())
    } else {
        Err(S3Error::ResourceOutOfScope {
            expected: prefix,
            found: to_sign.resource().to_owned(),
        })
    }
}

/// The base64 MD5 of one part's bytes. Per part, not per file: the captured
/// upload was a single part, which made the two indistinguishable, and
/// reusing a whole-file digest on part two would be rejected by S3 with a
/// digest mismatch after the bytes had already gone out.
#[must_use]
pub fn content_md5(bytes: &[u8]) -> String {
    let digest = Md5::digest(bytes);
    base64::engine::general_purpose::STANDARD.encode(digest)
}

/// One part's byte range within the whole payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartPlan {
    pub number: u32,
    pub start: usize,
    pub end: usize,
}

impl PartPlan {
    /// The part's own bytes. Returns `None` where the plan does not fit the
    /// payload, which cannot happen for a plan this module built but is not
    /// worth a panic to assert.
    #[must_use]
    pub fn slice<'a>(&self, payload: &'a [u8]) -> Option<&'a [u8]> {
        payload.get(self.start..self.end)
    }
}

/// Cuts a payload into parts. Always at least one part, even for a payload
/// smaller than `part_size`: the captured client ran initiate/PUT/complete
/// for a 224 KB file, so a small upload takes the same three hops as a large
/// one and there is no single-PUT path to fall back to.
pub fn plan_parts(total: usize, part_size: usize) -> Result<Vec<PartPlan>, S3Error> {
    if part_size == 0 {
        return Err(S3Error::Unplannable(
            "a part size of zero cuts nothing".to_owned(),
        ));
    }
    if total == 0 {
        return Err(S3Error::Unplannable(
            "an empty file has nothing to upload".to_owned(),
        ));
    }
    let mut parts = Vec::new();
    let mut start: usize = 0;
    let mut number: u32 = 1;
    while start < total {
        let end = start.checked_add(part_size).unwrap_or(total).min(total);
        parts.push(PartPlan { number, start, end });
        start = end;
        number = number.checked_add(1).ok_or_else(|| {
            S3Error::Unplannable("more parts than a part number can express".to_owned())
        })?;
    }
    Ok(parts)
}

/// The text of one XML element, found by its tags. S3's answers are small,
/// flat and machine-generated, so an anchored read is faithful where a parser
/// would be a dependency.
fn element(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let at = xml.find(&open)?;
    let from = at.checked_add(open.len())?;
    let rest = xml.get(from..)?;
    let to = rest.find(&close)?;
    Some(rest.get(..to)?.to_owned())
}

/// The upload id an `InitiateMultipartUploadResult` carries.
pub fn parse_upload_id(xml: &str) -> Result<String, S3Error> {
    element(xml, "UploadId")
        .ok_or_else(|| S3Error::Shape("the initiate answer carries no UploadId element".to_owned()))
}

/// The `CompleteMultipartUpload` body, listing every part in ascending order
/// with the ETag S3 returned for it, quotes included exactly as the header
/// carried them.
#[must_use]
pub fn complete_multipart_body(parts: &[(u32, String)]) -> String {
    let mut body = String::from("<CompleteMultipartUpload>");
    for (number, etag) in parts {
        let quoted = if etag.starts_with('"') {
            etag.clone()
        } else {
            format!("\"{etag}\"")
        };
        body.push_str("<Part><PartNumber>");
        body.push_str(&number.to_string());
        body.push_str("</PartNumber><ETag>");
        body.push_str(&quoted);
        body.push_str("</ETag></Part>");
    }
    body.push_str("</CompleteMultipartUpload>");
    body
}

/// Confirms a `CompleteMultipartUploadResult`. The completion is the only
/// point at which S3 states the object exists whole, so a 200 whose body is
/// something else is not a completed upload.
pub fn confirm_complete(xml: &str) -> Result<(), S3Error> {
    if xml.contains("<CompleteMultipartUploadResult") && element(xml, "ETag").is_some() {
        Ok(())
    } else {
        Err(S3Error::Shape(
            "the complete answer is not a CompleteMultipartUploadResult carrying an ETag"
                .to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        complete_multipart_body, confirm_complete, content_md5, parse_upload_id, plan_parts,
        sign_auth_scope, string_to_sign, S3Error, S3Operation, UploadTicket, PART_SIZE,
    };

    fn ticket() -> UploadTicket {
        UploadTicket::new(
            "live.digital.upload".to_owned(),
            "de07-00000000-2026-08-28/product/0123456789abcdef0123456789abcdef_000000001.png"
                .to_owned(),
        )
    }

    #[test]
    fn the_initiate_string_to_sign_matches_the_captured_shape() {
        let signable = string_to_sign(
            &ticket(),
            &S3Operation::Initiate,
            None,
            "image/png",
            "Fri, 28 Aug 2026 05:57:21 GMT",
        );
        assert_eq!(
            signable.as_str(),
            "POST\n\nimage/png\n\nx-amz-date:Fri, 28 Aug 2026 05:57:21 GMT\n\
             /live.digital.upload/de07-00000000-2026-08-28/product/\
             0123456789abcdef0123456789abcdef_000000001.png?uploads",
            "the Date slot is empty because x-amz-date carries the instant instead"
        );
    }

    #[test]
    fn a_part_string_to_sign_carries_the_digest_and_the_subresource() {
        let signable = string_to_sign(
            &ticket(),
            &S3Operation::UploadPart {
                part_number: 2,
                upload_id: "UPLOADID".to_owned(),
            },
            Some("KKbcrLiqOZ3XxlkXFVrrFw=="),
            "image/png",
            "Fri, 28 Aug 2026 05:57:22 GMT",
        );
        assert!(
            signable
                .as_str()
                .starts_with("PUT\nKKbcrLiqOZ3XxlkXFVrrFw==\nimage/png\n\n"),
            "the Content-MD5 is the second line and S3 rejects a mismatch, got {:?}",
            signable.as_str()
        );
        assert!(
            signable
                .resource()
                .ends_with("?partNumber=2&uploadId=UPLOADID"),
            "the subresource is part of what is signed, got {:?}",
            signable.resource()
        );
    }

    #[test]
    fn a_string_over_another_sellers_object_never_reaches_the_oracle() {
        let mine = ticket();
        let theirs = UploadTicket::new(
            "live.digital.upload".to_owned(),
            "ffff-99999999-2026-08-28/product/deadbeef.png".to_owned(),
        );
        let signable = string_to_sign(
            &theirs,
            &S3Operation::Initiate,
            None,
            "image/png",
            "Fri, 28 Aug 2026 05:57:21 GMT",
        );
        let refused = sign_auth_scope(&mine, &signable);
        let Err(S3Error::ResourceOutOfScope { expected, found }) = refused else {
            panic!("the oracle signs whatever it is given, so this must refuse: {refused:?}");
        };
        assert!(
            expected.contains("de07-00000000") && found.contains("ffff-99999999"),
            "the refusal names both objects, got expected {expected:?} found {found:?}"
        );
    }

    #[test]
    fn a_string_over_a_prefix_of_this_object_is_also_out_of_scope() {
        let mine = ticket();
        let sibling = UploadTicket::new(
            "live.digital.upload".to_owned(),
            "de07-00000000-2026-08-28/product/0123456789abcdef0123456789abcdef_000000001.png.bak"
                .to_owned(),
        );
        let signable = string_to_sign(
            &sibling,
            &S3Operation::Initiate,
            None,
            "image/png",
            "Fri, 28 Aug 2026 05:57:21 GMT",
        );
        assert!(
            sign_auth_scope(&mine, &signable).is_err(),
            "the guard must anchor on the whole key, not on a prefix of it"
        );
    }

    #[test]
    fn every_operation_this_module_builds_is_in_scope_for_its_own_ticket() {
        let ticket = ticket();
        for operation in [
            S3Operation::Initiate,
            S3Operation::UploadPart {
                part_number: 1,
                upload_id: "U".to_owned(),
            },
            S3Operation::Complete {
                upload_id: "U".to_owned(),
            },
        ] {
            let signable = string_to_sign(&ticket, &operation, None, "image/png", "date");
            assert_eq!(
                sign_auth_scope(&ticket, &signable),
                Ok(()),
                "{operation:?} is this ticket's own operation"
            );
        }
    }

    #[test]
    fn the_content_digest_is_the_captured_one() {
        // The captured PUT sent Content-MD5 for its own 223 791 bytes; this
        // asserts the encoding rather than that file's digest.
        assert_eq!(
            content_md5(b""),
            "1B2M2Y8AsgTpgAmY7PhCfg==",
            "base64 of the MD5 digest, which is the form the StringToSign signs"
        );
    }

    #[test]
    fn a_small_payload_still_takes_the_multipart_path() {
        let parts = plan_parts(10, PART_SIZE).expect("ten bytes are plannable");
        assert_eq!(
            parts.len(),
            1,
            "the capture ran initiate/PUT/complete for a file far under the part size"
        );
        assert_eq!(
            (
                parts.first().map(|part| part.number),
                parts.first().map(|part| part.end)
            ),
            (Some(1), Some(10)),
            "the single part covers the whole payload"
        );
    }

    #[test]
    fn a_payload_over_the_part_size_cuts_into_ordered_parts_that_tile_it() {
        let total = 12_000;
        let parts = plan_parts(total, 5_000).expect("the payload is plannable");
        assert_eq!(
            parts.iter().map(|part| part.number).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "part numbers ascend from one, which is the order the complete body lists"
        );
        assert_eq!(
            parts
                .iter()
                .map(|part| (part.start, part.end))
                .collect::<Vec<_>>(),
            vec![(0, 5_000), (5_000, 10_000), (10_000, 12_000)],
            "the parts tile the payload with no gap and no overlap"
        );
        let covered: usize = parts
            .iter()
            .map(|part| part.end.saturating_sub(part.start))
            .sum();
        assert_eq!(covered, total, "every byte belongs to exactly one part");
    }

    #[test]
    fn each_part_carries_its_own_digest_and_not_the_whole_files() {
        let payload: Vec<u8> = (0u8..=255).cycle().take(12_000).collect();
        let parts = plan_parts(payload.len(), 5_000).expect("the payload is plannable");
        let digests: Vec<String> = parts
            .iter()
            .filter_map(|part| part.slice(&payload))
            .map(content_md5)
            .collect();
        assert_eq!(digests.len(), 3, "one digest per part");
        assert!(
            digests
                .iter()
                .all(|digest| *digest != content_md5(&payload)),
            "a whole-file digest on part two is rejected after the bytes have gone out"
        );
    }

    #[test]
    fn an_empty_upload_is_refused_rather_than_sent_as_a_zero_part_multipart() {
        assert!(
            matches!(plan_parts(0, PART_SIZE), Err(S3Error::Unplannable(_))),
            "a complete body listing no parts is not an upload"
        );
    }

    #[test]
    fn the_initiate_answer_yields_its_upload_id() {
        let xml = "<?xml version=\"1.0\"?><InitiateMultipartUploadResult>\
                   <Bucket>live.digital.upload</Bucket><Key>k</Key>\
                   <UploadId>suTohsfLZzEA</UploadId></InitiateMultipartUploadResult>";
        assert_eq!(parse_upload_id(xml).as_deref(), Ok("suTohsfLZzEA"));
        assert!(
            parse_upload_id("<Error><Code>AccessDenied</Code></Error>").is_err(),
            "an error document is not an initiate result"
        );
    }

    #[test]
    fn the_complete_body_lists_every_part_with_its_quoted_etag() {
        let body = complete_multipart_body(&[(1, "\"aaa\"".to_owned()), (2, "bbb".to_owned())]);
        assert_eq!(
            body,
            "<CompleteMultipartUpload>\
             <Part><PartNumber>1</PartNumber><ETag>\"aaa\"</ETag></Part>\
             <Part><PartNumber>2</PartNumber><ETag>\"bbb\"</ETag></Part>\
             </CompleteMultipartUpload>",
            "S3 wants the ETag quoted, and the header may or may not have carried the quotes"
        );
    }

    #[test]
    fn a_completion_is_only_confirmed_by_the_result_document() {
        assert_eq!(
            confirm_complete(
                "<CompleteMultipartUploadResult><ETag>\"e-1\"</ETag></CompleteMultipartUploadResult>"
            ),
            Ok(())
        );
        assert!(
            confirm_complete("<Error><Code>InvalidPart</Code></Error>").is_err(),
            "S3 answers 200 with an error document, so the status is never the verdict"
        );
    }
}
