//! The TPT adapter: the seller's own catalogue and analytics off the two
//! GraphQL services, and the legacy CakePHP write path beside them.
//!
//! The read half is gated on the first-party-export capability, because an
//! enumeration-shaped read is exactly that tier and nothing else justifies
//! one. The write half is the eleven-hop create chain — form render, staged
//! S3 upload, two async jobs, then a multipart form navigation — followed by
//! a publishing edit. Both halves classify every hop on its own answer; no
//! hop trusts a status another hop produced.
//!
//! The write is deliberately draft-first. `status_user` is a form field
//! rather than a route, so a create leaves a draft and publishing is a second
//! submit, which is what makes an ambiguous create survivable.

use serde_json::Value;
use tam_marketplace::transport::{HttpRequest, HttpResponse, RequestBody, Transport};
use tam_marketplace::{
    AdapterError, AmbiguityCause, FetchReason, FieldSet, FileContent, FileSource, FirstPartyExport,
    FormId, FormSchemaFingerprint, IdempotencyKey, ImportedListing, ListingLocator,
    MarketplaceAdapter, ObservedListing, Pause, ProjectedListing, RemoteLifecycle, RemoteListingId,
    SchemaDrift, SubmitEvidence,
};
use tam_types::{
    ContentHash, FailureCode, FailureDetail, FieldKey, FileId, InventoryId, OrgId, Timestamp,
};

use crate::classify::{
    classify_form_page, classify_graphql_read, classify_queue_poll, classify_s3, classify_submit,
    classify_transport, classify_xhr_json, part_etag, queue_job, SubmitLanding,
};
use crate::endpoints::{
    self, AllTimeMetric, FormTarget, ResolvedStatsQuery, SignedS3Call, UploadReservation,
    CATALOGUE_PAGE_LIMIT, STATS_BATCH_MAX,
};
use crate::form::TptFormPage;
use crate::read_model::{self, ProductId, ResourceStat, TptCatalogueEntry};
use crate::s3::{self, AwsKeyId, S3Operation, S3Signature, UploadSlot, UploadTicket, PART_SIZE};
use crate::upload::{
    cache_buster, queue_exhausted, Hop, ProcessedHandle, QueuePayload, QueueState,
    ThumbnailCollection, UploadHandle, QUEUE_POLL_INTERVAL_MS, QUEUE_POLL_MAX,
};
use crate::write_model::{
    self, AuthorshipDeclaration, CreateSubmission, EditSubmission, StatusUser,
};

/// How many pages a catalogue walk will request before it refuses to
/// continue. The walk ends when it has collected `totalResultsCount` rows;
/// this cap only bounds a walk whose end never arrives.
pub const CATALOGUE_PAGE_MAX: u32 = 200;

/// The capability names the deferred methods report. Both are deferred by the
/// M7 plan's "Deferred, with owners" list: no capture contains a TPT file
/// download, and `ImportedListing` is Tes-shaped — licence tokens, GBP, age
/// ranges — where TPT's model is flat taxonomy tags with no licence, so
/// canonicalising TPT waits on the M6 import-run generalisation.
pub const BUNDLE_DOWNLOAD: &str = "tpt.download_resource_bundle";
/// See [`BUNDLE_DOWNLOAD`].
pub const IMPORT_CANONICALISATION: &str = "tpt.fetch_for_import";

/// The single TPT inventory, per the design's canonical inventory table.
pub struct TptAdapter<T, F, P> {
    transport: T,
    file_source: F,
    pause: P,
    page_limit: u32,
    part_size: usize,
    authorship: Option<AuthorshipDeclaration>,
}

fn not_first_party(what: &str) -> AdapterError {
    AdapterError::Rejected {
        code: FailureCode::Other,
        detail: FailureDetail(format!(
            "a {what} is justified only by the first-party-export capability"
        )),
    }
}

fn refuse_upload(detail: String) -> AdapterError {
    AdapterError::Rejected {
        code: FailureCode::UploadRejected,
        detail: FailureDetail(detail),
    }
}

impl<T: Transport, F: FileSource, P: Pause> TptAdapter<T, F, P> {
    #[must_use]
    pub const fn new(transport: T, file_source: F, pause: P) -> Self {
        Self {
            transport,
            file_source,
            pause,
            page_limit: CATALOGUE_PAGE_LIMIT,
            part_size: PART_SIZE,
            authorship: None,
        }
    }

    /// The TPT client hard-codes a hundred rows per page and this adapter
    /// follows it; the page size is a constructor argument so a recorded walk
    /// can be replayed over a handful of rows rather than a hundred.
    #[must_use]
    pub const fn with_page_limit(mut self, page_limit: u32) -> Self {
        self.page_limit = page_limit;
        self
    }

    /// The multipart cut. Five mebibytes on the wire; a smaller value lets a
    /// test drive the multi-part loop over a payload a fixture can carry,
    /// which no capture does.
    #[must_use]
    pub const fn with_part_size(mut self, part_size: usize) -> Self {
        self.part_size = part_size;
        self
    }

    /// Binds the seller's authorship attestation to this connection. A submit
    /// without one refuses, so the copyright declaration the form posts is
    /// always a declaration somebody actually made.
    #[must_use]
    pub fn attesting(mut self, authorship: AuthorshipDeclaration) -> Self {
        self.authorship = Some(authorship);
        self
    }

    #[must_use]
    pub const fn inventory(&self) -> InventoryId {
        InventoryId::Tpt
    }

    /// The transport as constructed, so a cassette-driven test can assert its
    /// recording was consumed whole.
    pub const fn transport(&self) -> &T {
        &self.transport
    }

    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, AdapterError> {
        self.transport
            .send(request)
            .await
            .map_err(classify_transport)
    }

    async fn read(&self, request: HttpRequest) -> Result<Value, AdapterError> {
        let response = self.send(request).await?;
        classify_graphql_read(&response)
    }

    /// The seller's own catalogue, whole. The walk is driven by
    /// `pageInfo.totalResultsCount` rather than by an empty page, because TPT
    /// reports the enumeration target on every page; a walk that cannot reach
    /// it refuses rather than returning a truncation an importer would mistake
    /// for the entire catalogue.
    pub async fn list_own_resources(
        &self,
        reason: &FetchReason,
    ) -> Result<Vec<TptCatalogueEntry>, AdapterError> {
        if !matches!(reason, FetchReason::FirstPartyExport { .. }) {
            return Err(not_first_party("catalogue read"));
        }
        let mut entries: Vec<TptCatalogueEntry> = Vec::new();
        let mut offset: u32 = 0;
        for _ in 0..CATALOGUE_PAGE_MAX {
            let body = self
                .read(endpoints::my_product_listings_request(
                    self.page_limit,
                    offset,
                ))
                .await?;
            let page = read_model::parse_catalogue_page(&body).map_err(|error| {
                AdapterError::Rejected {
                    code: FailureCode::VerificationMismatch,
                    detail: FailureDetail(error.to_string()),
                }
            })?;
            let rows = page.entries.len();
            let target = page.total_results;
            entries.extend(page.entries);
            if entries.len() as u64 >= target {
                return Ok(entries);
            }
            if rows == 0 {
                return Err(AdapterError::Rejected {
                    code: FailureCode::VerificationMismatch,
                    detail: FailureDetail(format!(
                        "the catalogue walk ran dry at {} of {target} rows; the result would be \
                         truncated",
                        entries.len()
                    )),
                });
            }
            offset = offset
                .checked_add(self.page_limit)
                .ok_or_else(|| AdapterError::Rejected {
                    code: FailureCode::Other,
                    detail: FailureDetail("the catalogue offset overflowed".to_owned()),
                })?;
        }
        Err(AdapterError::Rejected {
            code: FailureCode::Other,
            detail: FailureDetail(format!(
                "the catalogue walk hit its {CATALOGUE_PAGE_MAX}-page cap without reaching the \
                 reported total; the result would be truncated"
            )),
        })
    }

    fn check_batch(ids: &[ProductId]) -> Result<(), AdapterError> {
        if ids.is_empty() {
            return Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail(
                    "a statistics read names the resources it is about".to_owned(),
                ),
            });
        }
        if ids.len() > STATS_BATCH_MAX {
            return Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail(format!(
                    "{} resource ids exceeds the {STATS_BATCH_MAX} proven on the wire; the \
                     ceiling above it is untested",
                    ids.len()
                )),
            });
        }
        Ok(())
    }

    async fn stats(&self, request: HttpRequest) -> Result<Vec<ResourceStat>, AdapterError> {
        let body = self.read(request).await?;
        read_model::parse_stats_edges(&body).map_err(|error| AdapterError::Rejected {
            code: FailureCode::VerificationMismatch,
            detail: FailureDetail(error.to_string()),
        })
    }

    /// All-time totals for one metric over one batch of the seller's own
    /// products. Inherent rather than on the seam: TPT is the only platform
    /// whose analytics we can read, and a capability with one implementor is
    /// not yet a seam. Promotion waits for a second.
    pub async fn all_time_stats(
        &self,
        reason: &FetchReason,
        metric: AllTimeMetric,
        ids: &[ProductId],
    ) -> Result<Vec<ResourceStat>, AdapterError> {
        if !matches!(reason, FetchReason::FirstPartyExport { .. }) {
            return Err(not_first_party("statistics read"));
        }
        Self::check_batch(ids)?;
        self.stats(endpoints::all_time_stats_request(metric, ids))
            .await
    }

    /// Time-resolved totals for one metric over one batch, for a dashboard
    /// that plots a window rather than a lifetime. See [`Self::all_time_stats`]
    /// for why this is inherent.
    pub async fn resolved_stats(
        &self,
        reason: &FetchReason,
        query: ResolvedStatsQuery,
        ids: &[ProductId],
    ) -> Result<Vec<ResourceStat>, AdapterError> {
        if !matches!(reason, FetchReason::FirstPartyExport { .. }) {
            return Err(not_first_party("statistics read"));
        }
        Self::check_batch(ids)?;
        self.stats(endpoints::resolved_stats_request(query, ids))
            .await
    }

    /// One render of one product form. Read-only, and the sole source of the
    /// token triple every write replays.
    pub async fn form_page(&self, target: FormTarget) -> Result<TptFormPage, AdapterError> {
        let response = self.send(endpoints::form_page_request(target)).await?;
        classify_form_page(&response)
    }

    /// Reserves the S3 object key. The bucket and path come back from the
    /// server and are never computed here: the key embeds a shard the client
    /// cannot derive.
    async fn reserve_upload(
        &self,
        file: &FileContent,
        now: Timestamp,
    ) -> Result<(UploadTicket, UploadHandle), AdapterError> {
        let response = self
            .send(endpoints::upload_file_request(&UploadReservation {
                slot: UploadSlot::Product,
                file_name: &file.file_name,
                size: file.bytes.len(),
                last_modified_ms: now.0,
                item_id: None,
            }))
            .await?;
        let body = classify_xhr_json(&response)?;
        let text = |key: &str| {
            body.get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| refuse_upload(format!("upload_file answered without a {key}")))
        };
        Ok((
            UploadTicket::new(text("bucket")?, text("path")?),
            UploadHandle::new(text("key")?),
        ))
    }

    /// TPT's clock, in RFC 1123. Used verbatim as `x-amz-date` and inside
    /// every `StringToSign`: AWS rejects a skewed signature, and the server
    /// that mints the signature is the one whose clock decides.
    async fn server_time(&self, now: Timestamp) -> Result<String, AdapterError> {
        let response = self.send(endpoints::time_request(now.0)).await?;
        if response.status != 200 {
            return Err(refuse_upload(format!(
                "the server clock read answered {}",
                response.status
            )));
        }
        let text = response.text().trim().to_owned();
        if text.is_empty() {
            return Err(refuse_upload(
                "the server clock read answered an empty body".to_owned(),
            ));
        }
        Ok(text)
    }

    /// One signature from TPT's oracle. The scope guard runs inside the
    /// request builder, so a string naming another object is refused before
    /// a request value exists.
    async fn signature(&self, request: &SignRequest<'_>) -> Result<S3Signature, AdapterError> {
        let signable = s3::string_to_sign(
            request.ticket,
            &request.operation,
            request.content_md5.as_deref(),
            request.content_type,
            request.amz_date,
        );
        let built = endpoints::sign_auth_request(request.ticket, &signable, request.amz_date)
            .map_err(|error| refuse_upload(error.to_string()))?;
        let response = self.send(built).await?;
        if response.status != 200 {
            return Err(refuse_upload(format!(
                "the signing oracle answered {}",
                response.status
            )));
        }
        let signature = response.text().trim().to_owned();
        if signature.is_empty() {
            return Err(refuse_upload(
                "the signing oracle answered an empty signature".to_owned(),
            ));
        }
        Ok(S3Signature::new(signature))
    }

    async fn signed_s3(
        &self,
        call: &SignedS3Call<'_>,
        body: RequestBody,
    ) -> Result<HttpResponse, AdapterError> {
        self.send(endpoints::s3_request(call, body)).await
    }

    /// Initiate, then one signed PUT per part, then complete. Always all
    /// three, even for a payload smaller than one part: the captured client
    /// ran the full sequence for a 224 KB file and there is no single-PUT
    /// path to fall back to.
    ///
    /// Every part carries its own `Content-MD5`, and the digest is the second
    /// line of the string the oracle signs. The captured upload was one part,
    /// which made a per-part digest and a whole-file digest indistinguishable;
    /// reusing the whole-file digest on part two is rejected by S3 after the
    /// bytes have already gone out.
    async fn s3_multipart(
        &self,
        upload: &S3Upload<'_>,
        payload: &[u8],
    ) -> Result<(), AdapterError> {
        let initiate = S3Operation::Initiate;
        let signature = self
            .signature(&SignRequest {
                ticket: upload.ticket,
                operation: initiate.clone(),
                content_md5: None,
                content_type: upload.content_type,
                amz_date: upload.amz_date,
            })
            .await?;
        let started = self
            .signed_s3(
                &upload.call(&initiate, &signature, None),
                RequestBody::Empty,
            )
            .await?;
        let upload_id = s3::parse_upload_id(&classify_s3(&started)?)
            .map_err(|error| refuse_upload(error.to_string()))?;

        let plan = s3::plan_parts(payload.len(), upload.part_size)
            .map_err(|error| refuse_upload(error.to_string()))?;
        let mut completed: Vec<(u32, String)> = Vec::with_capacity(plan.len());
        for part in plan {
            let bytes = part.slice(payload).ok_or_else(|| {
                refuse_upload("a planned part fell outside the payload".to_owned())
            })?;
            let digest = s3::content_md5(bytes);
            let operation = S3Operation::UploadPart {
                part_number: part.number,
                upload_id: upload_id.clone(),
            };
            let signature = self
                .signature(&SignRequest {
                    ticket: upload.ticket,
                    operation: operation.clone(),
                    content_md5: Some(digest.clone()),
                    content_type: upload.content_type,
                    amz_date: upload.amz_date,
                })
                .await?;
            let response = self
                .signed_s3(
                    &upload.call(&operation, &signature, Some(&digest)),
                    RequestBody::Bytes(bytes.to_vec()),
                )
                .await?;
            classify_s3(&response)?;
            completed.push((part.number, part_etag(&response)?));
        }

        let operation = S3Operation::Complete { upload_id };
        let body = s3::complete_multipart_body(&completed);
        let signature = self
            .signature(&SignRequest {
                ticket: upload.ticket,
                operation: operation.clone(),
                content_md5: None,
                content_type: upload.content_type,
                amz_date: upload.amz_date,
            })
            .await?;
        let finished = self
            .signed_s3(
                &upload.call(&operation, &signature, None),
                RequestBody::Bytes(body.into_bytes()),
            )
            .await?;
        s3::confirm_complete(&classify_s3(&finished)?)
            .map_err(|error| refuse_upload(error.to_string()))
    }

    /// Polls one async job to its terminal state. The job id came from a
    /// response header, and the cadence is the client's own measured one.
    async fn await_job(
        &self,
        job: &crate::upload::QueueJob,
        key: IdempotencyKey,
        ordinal: u32,
    ) -> Result<QueuePayload, AdapterError> {
        for attempt in 0..QUEUE_POLL_MAX {
            let buster = cache_buster(
                key,
                Hop::QueuePoll {
                    job: ordinal,
                    attempt,
                }
                .ordinal(),
            );
            let response = self
                .send(endpoints::queue_results_request(job, &buster))
                .await?;
            let body = classify_xhr_json(&response)?;
            match classify_queue_poll(&body)? {
                QueueState::Complete(payload) => return Ok(payload),
                QueueState::Queued | QueueState::Running { .. } => {
                    self.pause.pause(QUEUE_POLL_INTERVAL_MS).await;
                }
            }
        }
        Err(queue_exhausted(QUEUE_POLL_MAX))
    }

    /// The whole out-of-band upload: reserve, sign and store the bytes, then
    /// walk the two async jobs that turn a staged object into the two handles
    /// the product form consumes.
    async fn stage_file(
        &self,
        staging: &Staging<'_>,
    ) -> Result<(ProcessedHandle, ThumbnailCollection), AdapterError> {
        let file = staging.file;
        let (ticket, staged) = self.reserve_upload(file, staging.now).await?;
        let amz_date = self.server_time(staging.now).await?;
        self.s3_multipart(
            &S3Upload {
                ticket: &ticket,
                key_id: staging.page.aws_key_id(),
                amz_date: &amz_date,
                content_type: &file.content_type,
                part_size: self.part_size,
            },
            &file.bytes,
        )
        .await?;

        let enqueued = self
            .send(endpoints::process_file_request(
                &staged,
                None,
                &cache_buster(staging.key, Hop::ProcessFile.ordinal()),
            ))
            .await?;
        classify_xhr_json(&enqueued)?;
        let processed = match self
            .await_job(&queue_job(&enqueued)?, staging.key, 1)
            .await?
        {
            QueuePayload::Processed(handle) => handle,
            QueuePayload::Thumbnails(_) => {
                return Err(refuse_upload(
                    "the processing job answered a thumbnail collection".to_owned(),
                ))
            }
        };

        let thumbs = self
            .send(endpoints::generate_thumbs_request(
                &processed,
                None,
                &cache_buster(staging.key, Hop::GenerateThumbs.ordinal()),
            ))
            .await?;
        classify_xhr_json(&thumbs)?;
        let collection = match self.await_job(&queue_job(&thumbs)?, staging.key, 2).await? {
            QueuePayload::Thumbnails(collection) => collection,
            QueuePayload::Processed(_) => {
                return Err(refuse_upload(
                    "the thumbnail job answered a processed asset key".to_owned(),
                ))
            }
        };
        Ok((processed, collection))
    }

    fn attestation(&self) -> Result<&AuthorshipDeclaration, AdapterError> {
        self.authorship.as_ref().ok_or_else(|| {
            refuse_upload(
                "this connection carries no authorship attestation, and the product form's \
                 copyright declaration is the seller's statement that the work is their own; it \
                 is not a constant this connector may supply on their behalf"
                    .to_owned(),
            )
        })
    }

    /// Exactly one file into the product slot. The preview, video and manual
    /// thumbnail slots are named by the form's config and exercised by no
    /// capture, so a policy for them would be invention.
    fn sole_file(fields: &FieldSet) -> Result<FileId, AdapterError> {
        match fields.files.as_slice() {
            [only] => Ok(*only),
            other => Err(refuse_upload(format!(
                "a TPT create takes exactly one file into the product slot and this projection \
                 carries {}; the preview, video and thumbnail slots are uncaptured",
                other.len()
            ))),
        }
    }

    async fn fetch_sole_file(&self, fields: &FieldSet) -> Result<FileContent, AdapterError> {
        let id = Self::sole_file(fields)?;
        self.file_source
            .fetch(id)
            .await
            .map_err(|error| refuse_upload(format!("file source: {error:?}")))
    }

    /// The publishing edit: the same 48-field form the captured edit posted,
    /// with the status selector moved to live.
    ///
    /// A read-modify-write whose read is the edit render — the tokens and the
    /// existing thumbnail handles, which an edit that dropped them would drop
    /// from the product — and whose model is the declared intent this
    /// connector created the draft from.
    pub async fn publish(
        &self,
        product: ProductId,
        fields: &FieldSet,
    ) -> Result<SubmitLanding, AdapterError> {
        let authorship = self.attestation()?;
        let listing = write_model::listing_from_field_set(fields)?;
        let target = FormTarget::EditDigital(product);
        let page = self.form_page(target).await?;
        let body = write_model::edit_fields(&EditSubmission {
            tokens: page.tokens(),
            listing: &listing,
            thumbs: page.thumbs(),
            status: StatusUser::Live,
            authorship,
        });
        let response = self
            .send(endpoints::submit_form_request(target, body))
            .await?;
        let landing = classify_submit(&response)?;
        if landing.product == product {
            Ok(landing)
        } else {
            // The edit route names the product it edits, so a redirect to a
            // different one is not something this flow can reconcile.
            Err(AdapterError::Ambiguous(AmbiguityCause::NoDurableIdentifier))
        }
    }
}

/// One signing request. A struct rather than five arguments, which is also
/// what keeps the string, the ticket and the operation travelling together.
struct SignRequest<'a> {
    ticket: &'a UploadTicket,
    operation: S3Operation,
    content_md5: Option<String>,
    content_type: &'a str,
    amz_date: &'a str,
}

/// The invariant part of one multipart upload.
struct S3Upload<'a> {
    ticket: &'a UploadTicket,
    key_id: &'a AwsKeyId,
    amz_date: &'a str,
    content_type: &'a str,
    part_size: usize,
}

impl<'a> S3Upload<'a> {
    fn call(
        &self,
        operation: &'a S3Operation,
        signature: &'a S3Signature,
        content_md5: Option<&'a str>,
    ) -> SignedS3Call<'a> {
        SignedS3Call {
            ticket: self.ticket,
            operation,
            key_id: self.key_id,
            signature,
            amz_date: self.amz_date,
            content_type: self.content_type,
            content_md5,
        }
    }
}

/// What one file's staging needs: the bytes, the render that published the
/// AWS key id, the attempt's idempotency key and the driver's clock reading.
struct Staging<'a> {
    file: &'a FileContent,
    page: &'a TptFormPage,
    key: IdempotencyKey,
    now: Timestamp,
}

/// A fingerprint over the form's declared field-name set.
///
/// `md-5` is the only digest this crate carries and a fingerprint is
/// thirty-two bytes wide, so the two halves are domain-separated digests of
/// the same input. This detects a form whose shape moved; it is not a claim
/// about resisting an adversary who chooses the field names.
fn fingerprint(names: &[String]) -> FormSchemaFingerprint {
    use md5::{Digest as _, Md5};
    let joined = names.join("|");
    let low: [u8; 16] = Md5::digest(format!("tpt.form.lo\u{0}{joined}").as_bytes()).into();
    let high: [u8; 16] = Md5::digest(format!("tpt.form.hi\u{0}{joined}").as_bytes()).into();
    let mut bytes = [0_u8; 32];
    for (slot, byte) in bytes.iter_mut().zip(low.iter().chain(high.iter())) {
        *slot = *byte;
    }
    FormSchemaFingerprint(ContentHash(bytes))
}

impl<T: Transport, F: FileSource, P: Pause> MarketplaceAdapter for TptAdapter<T, F, P> {
    fn inventory(&self) -> InventoryId {
        InventoryId::Tpt
    }

    fn project_fields(&self, listing: &ProjectedListing) -> Result<FieldSet, AdapterError> {
        write_model::project_fields(listing)
    }

    /// A genuine structural probe: one GET of the create form, and nothing
    /// else. No draft is created, nothing is submitted and nothing is
    /// deleted, which is what the read-only probe tier permits.
    ///
    /// The form states its own shape in `data[_Token][unlocked]`, a
    /// pipe-joined list of every field path the render accepts. A path this
    /// adapter writes that the render no longer declares is drift; a path the
    /// render declares that we do not write is not, because the list is a
    /// superset by construction — the captured create itself omitted several
    /// of its members.
    async fn assert_form_schema(
        &self,
        _org: OrgId,
        form: FormId,
    ) -> Result<FormSchemaFingerprint, AdapterError> {
        let page = self.form_page(FormTarget::CreateDigital).await?;
        let declared = page.tokens().unlocked_field_names();
        let written = write_model::written_field_paths();
        let missing: Vec<String> = written
            .iter()
            .filter(|path| !declared.contains(path))
            .cloned()
            .collect();
        if missing.is_empty() {
            Ok(fingerprint(&declared))
        } else {
            Err(AdapterError::SchemaDrift(Box::new(SchemaDrift {
                form,
                expected: fingerprint(&written),
                observed: fingerprint(&declared),
                added: Vec::new(),
                removed: missing,
            })))
        }
    }

    /// The eleven-hop create, in the order the capture records it: render the
    /// form, stage the file, then post the product.
    ///
    /// The token set ages across the upload, exactly as it did in the capture
    /// — nearly three minutes there — because the render is also the only
    /// source of the AWS key id the S3 calls need, so it must come first.
    ///
    /// Everything before the final POST is safe to refuse plainly: no product
    /// form has been posted, so nothing was created. The final POST is the
    /// one hop whose failure is an ambiguity.
    async fn submit(
        &self,
        _org: OrgId,
        key: IdempotencyKey,
        fields: FieldSet,
        now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        let authorship = self.attestation()?;
        let listing = write_model::listing_from_field_set(&fields)?;
        let file = self.fetch_sole_file(&fields).await?;
        let target = FormTarget::CreateDigital;
        let page = self.form_page(target).await?;
        let (product, thumbs) = self
            .stage_file(&Staging {
                file: &file,
                page: &page,
                key,
                now,
            })
            .await?;
        let body = write_model::create_fields(&CreateSubmission {
            tokens: page.tokens(),
            listing: &listing,
            product: &product,
            thumbs_collection_key: thumbs.collection_key(),
            authorship,
        });
        let response = self
            .send(endpoints::submit_form_request(target, body))
            .await?;
        let landing = classify_submit(&response)?;
        Ok(SubmitEvidence {
            http_status: Some(response.status),
            response_body_digest: None,
            landed_on_route: Some(landing.location.clone()),
            landed: Some(RemoteListingId::Tpt {
                product_id: landing.product.0,
            }),
            observed_lag: false,
        })
    }

    /// One product by the identifier a receipt already holds, through the
    /// same `MyProductListings` operation the catalogue walk uses: its
    /// captured query text declares a `resourceIds` variable, and naming one
    /// product is the narrowest read that query supports.
    async fn read_back(
        &self,
        _org: OrgId,
        locator: ListingLocator,
        reason: FetchReason,
        observed_at: Timestamp,
    ) -> Result<ObservedListing, AdapterError> {
        match reason {
            FetchReason::VerifyWrite { .. }
            | FetchReason::VerifyAttempt { .. }
            | FetchReason::PollLifecycle { .. }
            | FetchReason::FirstPartyExport { .. } => {}
            // The probe resolves selectors against a form and reads no
            // listing; a listing read under it would be a read with no
            // justification behind it.
            FetchReason::StructuralProbe { .. } => {
                return Err(not_first_party("listing read-back"))
            }
        }
        let product = product_from_locator(&locator)?;
        let body = self.read(endpoints::product_by_id_request(product)).await?;
        let page =
            read_model::parse_catalogue_page(&body).map_err(|error| AdapterError::Rejected {
                code: FailureCode::VerificationMismatch,
                detail: FailureDetail(error.to_string()),
            })?;
        let entry = page
            .entries
            .into_iter()
            .find(|entry| entry.id == product)
            .ok_or(AdapterError::Ambiguous(
                AmbiguityCause::ReadBackIndeterminate,
            ))?;
        let taxonomy = serde_json::json!({
            "tags": entry.taxonomy_tags,
            "categories": entry
                .categories
                .iter()
                .map(|shelf| shelf.id.clone())
                .collect::<Vec<_>>(),
        });
        let fields = vec![
            (FieldKey::Title, entry.name.clone()),
            (FieldKey::Taxonomy, taxonomy.to_string()),
            (
                FieldKey::Price,
                serde_json::json!({
                    "free": entry.is_free,
                    "minorUnits": entry.price.minor_units,
                    "symbol": entry.price.symbol,
                })
                .to_string(),
            ),
        ];
        // Every one of the 154 products read `ACTIVE`, so that is the only
        // live state on file. Anything else reads as not-yet-live, which is
        // the side that never claims a draft is published.
        let lifecycle = if entry.status.as_deref() == Some("ACTIVE") {
            RemoteLifecycle::Live { since: observed_at }
        } else {
            RemoteLifecycle::Draft
        };
        Ok(ObservedListing {
            id: RemoteListingId::Tpt {
                product_id: product.0,
            },
            fields,
            lifecycle,
        })
    }
}

fn product_from_locator(locator: &ListingLocator) -> Result<ProductId, AdapterError> {
    match locator {
        ListingLocator::Durable(RemoteListingId::Tpt { product_id }) => Ok(ProductId(*product_id)),
        ListingLocator::Durable(RemoteListingId::Tes { .. } | RemoteListingId::Etsy { .. }) => {
            Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail("a non-TPT identifier reached the TPT adapter".to_owned()),
            })
        }
        // Marker reconciliation over `MyProductListings` is the top M7
        // follow-up: it needs a fetch reason that justifies an enumeration
        // read during a write, which is a seam extension of its own.
        ListingLocator::Marker { .. } => Err(AdapterError::Rejected {
            code: FailureCode::Other,
            detail: FailureDetail(
                "marker reconciliation against MyProductListings is not implemented yet".to_owned(),
            ),
        }),
    }
}

impl<T: Transport, F: FileSource, P: Pause> FirstPartyExport for TptAdapter<T, F, P> {
    type CatalogueEntry = TptCatalogueEntry;
    type Resource = ProductId;

    fn list_own_resources(
        &self,
        reason: &FetchReason,
    ) -> impl core::future::Future<Output = Result<Vec<TptCatalogueEntry>, AdapterError>> + Send
    {
        Self::list_own_resources(self, reason)
    }

    fn download_resource_bundle(
        &self,
        _reason: &FetchReason,
        _id: ProductId,
    ) -> impl core::future::Future<Output = Result<Vec<u8>, AdapterError>> + Send {
        core::future::ready(Err(AdapterError::Uncaptured {
            capability: BUNDLE_DOWNLOAD,
        }))
    }

    fn fetch_for_import(
        &self,
        _reason: &FetchReason,
        _id: ProductId,
    ) -> impl core::future::Future<Output = Result<ImportedListing, AdapterError>> + Send {
        core::future::ready(Err(AdapterError::Uncaptured {
            capability: IMPORT_CANONICALISATION,
        }))
    }
}
