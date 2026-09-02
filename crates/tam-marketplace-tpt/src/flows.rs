//! The TPT adapter: the seller's own catalogue and analytics off the two
//! GraphQL services, and the legacy CakePHP write path beside them.
//!
//! The read half is gated on the first-party-export capability, because an
//! enumeration-shaped read is exactly that tier and nothing else justifies
//! one. The write half is the thirteen-hop create chain — form render, staged
//! S3 upload, two async jobs, then a multipart form navigation — the edit that
//! updates or publishes what it created, and the one-hop mutation that removes
//! it. Both halves classify every hop on its own answer; no hop trusts a
//! status another hop produced.
//!
//! The write is deliberately draft-first. `status_user` is a form field
//! rather than a route, so a create leaves a draft and publishing is a second
//! submit, which is what makes an ambiguous create survivable.

use serde_json::Value;
use sha2::{Digest, Sha256};
use tam_marketplace::transport::{
    HttpRequest, HttpResponse, RequestBody, ResponseHeader, Transport,
};
use tam_marketplace::{
    AdapterError, AmbiguityCause, FetchReason, FieldSet, FileContent, FileSource, FirstPartyExport,
    FormId, FormSchemaFingerprint, IdempotencyKey, ImportedListing, ListingLocator, ListingState,
    MarketplaceAdapter, ObservedListing, Pause, ProjectedListing, RemoteLifecycle, RemoteListingId,
    RemovalPlan, RevisePlan, SchemaDrift, SubmitEvidence,
};
use tam_types::{
    ContentHash, CopyFormat, CurrencyRule, FailureCode, FailureDetail, FieldKey, FileId,
    ImportedPrice, ImportedTerm, InventoryId, Timestamp,
};

use crate::classify::{
    classify_edit_submit, classify_form_page, classify_graphql_read, classify_pre_write_transport,
    classify_queue_poll, classify_read_bytes, classify_s3, classify_submit, classify_text_hop,
    classify_transport, classify_xhr_json, part_etag, queue_job, uncleared_download, SubmitLanding,
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

/// The symbol TPT renders every amount with, and the only one it renders:
/// the marketplace sells in one currency.
const DOLLAR: &str = "$";

/// A paid price, denominated from the inventory's own rule rather than from
/// the symbol beside it.
///
/// TPT sells in USD and offers the seller no other currency, confirmed in the
/// account on 2026-08-29, so the denomination is a fact of the marketplace.
/// The symbol is still read: a non-dollar symbol on a dollar-only marketplace
/// contradicts what that rule asserts, and naming the contradiction is worth
/// more than redenominating around it.
fn imported_price(price: &read_model::TptPrice) -> Result<ImportedPrice, AdapterError> {
    if price.symbol != DOLLAR {
        return Err(AdapterError::Rejected {
            code: FailureCode::VerificationMismatch,
            detail: FailureDetail(format!(
                "TPT prices in USD and rendered this amount with {:?}; the currency rule and \
                 the wire disagree, which is not something to resolve by picking one",
                price.symbol
            )),
        });
    }
    let CurrencyRule::Fixed(currency) = InventoryId::Tpt.currency_rule() else {
        return Err(AdapterError::Rejected {
            code: FailureCode::Other,
            detail: FailureDetail(
                "a paid TPT product needs a fixed currency and the inventory declares none"
                    .to_owned(),
            ),
        });
    };
    Ok(ImportedPrice::Paid {
        minor_units: price.minor_units,
        denomination: currency.code().to_owned(),
    })
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

    /// Every hop before the final POST. A lost response here is reported as
    /// safe to retry, because none of these hops can have created a product;
    /// see [`classify_pre_write_transport`].
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, AdapterError> {
        self.transport
            .send(request)
            .await
            .map_err(classify_pre_write_transport)
    }

    /// The hops whose lost answer leaves the write state unknown: the final
    /// product POST, the publishing edit's POST, and the reads a receipt is
    /// settled from.
    async fn send_ambiguous_on_loss(
        &self,
        request: HttpRequest,
    ) -> Result<HttpResponse, AdapterError> {
        self.transport
            .send(request)
            .await
            .map_err(classify_transport)
    }

    async fn read(&self, request: HttpRequest) -> Result<Value, AdapterError> {
        let response = self.send_ambiguous_on_loss(request).await?;
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

    /// TPT's clock, in RFC 1123. Used verbatim as `x-amz-date` and inside the
    /// `StringToSign` of the one call it was read for: AWS rejects a skewed
    /// signature, and the server that mints the signature is the one whose
    /// clock decides.
    ///
    /// `requestTime` is the caller's own clock reading, which advances
    /// between calls in the capture and is what keeps two reads from being
    /// one cacheable URL. This connector holds a single clock reading per
    /// attempt, so the step ordinal supplies the same distinctness.
    async fn server_time(&self, now: Timestamp, step: u32) -> Result<String, AdapterError> {
        let response = self
            .send(endpoints::time_request(
                now.0.saturating_add(i64::from(step)),
            ))
            .await?;
        classify_text_hop(&response, "the server clock read")
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
        classify_text_hop(&response, "the signing oracle").map(S3Signature::new)
    }

    /// One signed S3 call: TPT's clock, a signature over that instant, then
    /// the call carrying the same instant.
    ///
    /// The clock is read per call rather than once per upload. AWS rejects a
    /// SigV2 signature whose `x-amz-date` sits outside a fifteen-minute skew
    /// window, and a multi-part upload of a product the form accepts up to
    /// four gibibytes of outlives that window; the captured upload was a
    /// single 224 KB part, which made one instant and one per part
    /// indistinguishable.
    async fn signed_s3(
        &self,
        upload: &S3Upload<'_>,
        step: &S3Step<'_>,
        body: RequestBody,
    ) -> Result<HttpResponse, AdapterError> {
        let amz_date = self.server_time(upload.now, step.ordinal).await?;
        let signature = self
            .signature(&SignRequest {
                ticket: upload.ticket,
                operation: step.operation.clone(),
                content_md5: step.content_md5.map(str::to_owned),
                content_type: upload.content_type,
                amz_date: &amz_date,
            })
            .await?;
        let call = SignedS3Call {
            ticket: upload.ticket,
            operation: step.operation,
            key_id: upload.key_id,
            signature: &signature,
            amz_date: &amz_date,
            content_type: upload.content_type,
            content_md5: step.content_md5,
        };
        self.send(endpoints::s3_request(&call, body)).await
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
        let started = self
            .signed_s3(
                upload,
                &S3Step {
                    operation: &initiate,
                    content_md5: None,
                    ordinal: INITIATE_STEP,
                },
                RequestBody::Empty,
            )
            .await?;
        let upload_id = s3::parse_upload_id(&classify_s3(&started)?)
            .map_err(|error| refuse_upload(error.to_string()))?;

        let plan = s3::plan_parts(payload.len(), upload.part_size)
            .map_err(|error| refuse_upload(error.to_string()))?;
        let parts = u32::try_from(plan.len())
            .map_err(|_| refuse_upload("the part plan outran a part number".to_owned()))?;
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
            let response = self
                .signed_s3(
                    upload,
                    &S3Step {
                        operation: &operation,
                        content_md5: Some(&digest),
                        ordinal: part.number,
                    },
                    RequestBody::Bytes(bytes.to_vec()),
                )
                .await?;
            classify_s3(&response)?;
            completed.push((part.number, part_etag(&response)?));
        }

        let operation = S3Operation::Complete { upload_id };
        let body = s3::complete_multipart_body(&completed);
        let finished = self
            .signed_s3(
                upload,
                &S3Step {
                    operation: &operation,
                    content_md5: None,
                    ordinal: complete_step(parts),
                },
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
        self.s3_multipart(
            &S3Upload {
                ticket: &ticket,
                key_id: staging.page.aws_key_id(),
                content_type: &file.content_type,
                part_size: self.part_size,
                now: staging.now,
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

    /// Lifts the form-page scrape's own drift verdict into the seam's drift
    /// report. `classify_form_page` states the failure as
    /// `FormSchemaDrift`; the report is what carries it to the drift path,
    /// and the anchor the scrape named travels as the removed member because
    /// a render that carries no anchor declares no field either.
    fn preflight_drift(error: AdapterError, form: FormId, written: &[String]) -> AdapterError {
        if let AdapterError::Rejected {
            code: FailureCode::FormSchemaDrift,
            ref detail,
        } = error
        {
            return AdapterError::SchemaDrift(Box::new(SchemaDrift {
                form,
                expected: fingerprint(written),
                observed: fingerprint(&[]),
                added: Vec::new(),
                removed: vec![detail.0.clone()],
            }));
        }
        error
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

    /// The edit submit: the same form the captured edit posted — forty-eight
    /// fields there, and one per tag and shelf here — with this field set's
    /// values in place and the status the caller states.
    ///
    /// A read-modify-write whose read is the edit render — the tokens and the
    /// existing thumbnail handles, which an edit that dropped them would drop
    /// from the product — and whose model is the declared intent the caller
    /// wants the product to carry. Changing a description is this call with a
    /// changed field set; publishing is this call with the status moved.
    ///
    /// The status is stated rather than preserved because no render this
    /// connector scrapes carries `status_user`, and an edit is a full
    /// replace: something has to say which side of the draft line the product
    /// lands on, and a caller that means to keep the current one reads it
    /// back first.
    pub async fn update(
        &self,
        product: ProductId,
        fields: &FieldSet,
        status: StatusUser,
    ) -> Result<SubmitLanding, AdapterError> {
        self.post_edit(product, fields, status)
            .await
            .map(|(landing, _)| landing)
    }

    /// [`Self::update`] keeping the response beside the landing, so the
    /// trait's `revise` can state the status and the body digest its own
    /// write produced rather than re-deriving them from a second read.
    ///
    /// The inline `landing.product == product` check stays: it reads the
    /// write's own response, which is classification and the adapter's job,
    /// where a read of a lagging route would be verification and the
    /// driver's.
    async fn post_edit(
        &self,
        product: ProductId,
        fields: &FieldSet,
        status: StatusUser,
    ) -> Result<(SubmitLanding, HttpResponse), AdapterError> {
        let authorship = self.attestation()?;
        let listing = write_model::listing_from_field_set(fields)?;
        let target = FormTarget::EditDigital(product);
        let page = self.form_page(target).await?;
        let body = write_model::edit_fields(&EditSubmission {
            tokens: page.tokens(),
            listing: &listing,
            thumbs: page.thumbs(),
            status,
            authorship,
        });
        let response = self
            .send_ambiguous_on_loss(endpoints::submit_form_request(target, body))
            .await?;
        let landing = classify_edit_submit(&response, &target.path())?;
        if landing.product == product {
            Ok((landing, response))
        } else {
            // The edit route names the product it edits, so a redirect to a
            // different one is not something this flow can reconcile.
            Err(AdapterError::Ambiguous(AmbiguityCause::NoDurableIdentifier))
        }
    }

    /// Publishing is [`Self::update`] with the status selector moved to live.
    pub async fn publish(
        &self,
        product: ProductId,
        fields: &FieldSet,
    ) -> Result<SubmitLanding, AdapterError> {
        self.update(product, fields, StatusUser::Live).await
    }

    /// Removes one of the seller's own products, through the captured
    /// `RemoveResource` mutation.
    ///
    /// The mutation answers with the deleted product's own id, and that echo
    /// is the whole confirmation: nothing else in the answer distinguishes a
    /// delete that happened from one that did not. An answer that names
    /// another product, or names none, leaves this product's fate unsettled
    /// rather than refused — the mutation was posted, and a refusal here
    /// would be a claim the record survives that nothing observed.
    pub async fn delete(&self, product: ProductId) -> Result<(), AdapterError> {
        self.post_remove(product).await.map(drop)
    }

    /// [`Self::delete`] keeping the raw answer, so the trait's `remove` can
    /// digest the bytes the mutation actually returned. The id echo is the
    /// whole confirmation either way and stays here, because it reads the
    /// write's own response rather than a route that lags it.
    async fn post_remove(&self, product: ProductId) -> Result<(ProductId, Vec<u8>), AdapterError> {
        let response = self
            .send_ambiguous_on_loss(endpoints::remove_resource_request(product))
            .await?;
        let body = classify_graphql_read(&response)?;
        let deleted = read_model::parse_resource_delete(&body)
            .map_err(|_| AdapterError::Ambiguous(AmbiguityCause::ReadBackIndeterminate))?;
        if deleted == product {
            Ok((deleted, response.body))
        } else {
            Err(AdapterError::Ambiguous(AmbiguityCause::NoDurableIdentifier))
        }
    }
}

impl<T: Transport, F: FileSource, P: Pause> TptAdapter<T, F, P> {
    /// The seller's own copy of one of their products: the bytes behind the
    /// Download control on its product page.
    ///
    /// Two hops. The catalogue read names the product's `canonicalSlug`,
    /// which is what `downloadurl` is built from, and the download itself is
    /// a session-authenticated document navigation to `/Download/{slug}-{id}`
    /// -- a plain anchor on the page, with no minted token to reproduce.
    ///
    /// The redirect is followed explicitly rather than by the client, whose
    /// policy is `none` so the write path can read a product id out of a
    /// submit's `Location`. That is the better arrangement here too: a signed
    /// second hop stays visible in the cassette instead of disappearing
    /// inside reqwest, and an off-origin one is refused rather than followed,
    /// which is what keeps the seller's cookies on the marketplace.
    ///
    /// What is live-proven and what is not, stated plainly. The entry point,
    /// the identifier and the payload's declared shape are read from a
    /// capture of the product page. The download hop itself is not: a
    /// 2026-08-29 probe of the founder's own product answered `302` to the
    /// sign-in gate for a cookie jar that authenticates every GraphQL read
    /// and the entire write path, and navigation headers did not change it.
    /// So this route is gated on a browser clearance a server-side jar does
    /// not hold, TPT-as-source sync ships on the operator-manifest path
    /// instead, and what is written here is the flow as the wire format
    /// determines it -- correct the moment a session carries that clearance,
    /// and refusing legibly until then.
    pub async fn download_resource_bundle(
        &self,
        reason: &FetchReason,
        id: ProductId,
    ) -> Result<Vec<u8>, AdapterError> {
        if !matches!(reason, FetchReason::FirstPartyExport { .. }) {
            return Err(not_first_party("file download"));
        }
        let body = self.read(endpoints::product_by_id_request(id)).await?;
        let page =
            read_model::parse_catalogue_page(&body).map_err(|error| AdapterError::Rejected {
                code: FailureCode::VerificationMismatch,
                detail: FailureDetail(error.to_string()),
            })?;
        let entry = page
            .entries
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AdapterError::Rejected {
                code: FailureCode::PreconditionElementAbsent,
                detail: FailureDetail(format!(
                    "product {} is not in the seller's own catalogue, and this read downloads \
                     nothing else",
                    id.0
                )),
            })?;
        let first = self
            .send(endpoints::download_bundle_request(
                &entry.canonical_slug,
                id,
            ))
            .await?;
        let response = match first.header(ResponseHeader::Location) {
            None => first,
            Some(location) => match endpoints::download_redirect(location) {
                endpoints::DownloadRedirect::Authorization => {
                    return Err(uncleared_download(
                        "the download redirected to the sign-in gate",
                    ))
                }
                // No capture carries a signed hop, so its shape is unknown;
                // what is known is that it would leave the origin, and the
                // transport refuses a session request to any other host.
                // Inventing the request that would satisfy it is not a
                // substitute for capturing one.
                endpoints::DownloadRedirect::OffOrigin(url) => {
                    return Err(AdapterError::Rejected {
                        code: FailureCode::UnexpectedOrigin,
                        detail: FailureDetail(format!(
                            "the download redirected off the origin to {url:?}; no capture \
                             carries a signed download hop, so what it needs is unknown"
                        )),
                    })
                }
                endpoints::DownloadRedirect::SameOrigin(url) => {
                    self.send(endpoints::download_redirect_request(url)).await?
                }
            },
        };
        classify_read_bytes(&response).map(<[u8]>::to_vec)
    }

    /// The first-party import read: the seller's own product whole, through
    /// the query the edit form itself issues. Refuses any reason but
    /// `FirstPartyExport`, because this is the tier-one capability.
    ///
    /// Every value travels untagged. TPT's 358 facets are one flat namespace
    /// where a grade, a subject, a resource type and an audience are the same
    /// kind of thing, and which axis a slug answers is a fact of the seeded
    /// relation rather than of the array it came out of; tagging here would
    /// assert an axis binding this adapter cannot know.
    ///
    /// `rights` is `None`, and that is the measured fact rather than a gap:
    /// the eight HAR captures and the vocabulary poll between them reached
    /// every field either write posts and every field the read returns, and
    /// none of them is a licence. The registry records the same absence.
    pub async fn fetch_for_import(
        &self,
        reason: &FetchReason,
        id: ProductId,
    ) -> Result<ImportedListing, AdapterError> {
        if !matches!(reason, FetchReason::FirstPartyExport { .. }) {
            return Err(not_first_party("first-party import read"));
        }
        let response = self
            .send(endpoints::upload_page_product_request(id))
            .await?;
        let body = classify_graphql_read(&response)?;
        let product = read_model::parse_upload_page_product(&body, id).map_err(|error| {
            AdapterError::Rejected {
                code: FailureCode::VerificationMismatch,
                detail: FailureDetail(error.to_string()),
            }
        })?;

        let term = |slug: &str| ImportedTerm {
            inventory: InventoryId::Tpt,
            kind: None,
            segments: vec![slug.to_owned()],
            native_id: Some(slug.to_owned()),
        };
        let mut native: Vec<ImportedTerm> =
            product.taxonomy_tags.iter().map(|tag| term(tag)).collect();
        for shelf in &product.categories {
            native.push(ImportedTerm {
                inventory: InventoryId::Tpt,
                kind: None,
                segments: vec![shelf.name.clone()],
                native_id: Some(shelf.id.clone()),
            });
        }

        Ok(ImportedListing {
            remote: id.remote(),
            title: product.name,
            body: product.description,
            // TPT stores and returns the body as HTML, which is what makes a
            // TES-to-TPT sync a format question rather than a copy.
            body_format: CopyFormat::Html,
            native,
            rights: None,
            price: if product.is_free {
                ImportedPrice::Free
            } else {
                imported_price(&product.price)?
            },
            state: product
                .status
                .as_deref()
                .and_then(listing_state_from_status),
        })
    }
}

/// The draft line as TPT's own `status` states it, and `None` for anything
/// else.
///
/// Deliberately stricter than `read_back`, whose lenient else-arm reads an
/// unknown status as Draft. That leniency is right for a comparison and wrong
/// here: a read-back that guesses Draft is a comparison, while a removal that
/// guesses Draft posts a delete to the wrong route. Two values are on file
/// across 908 observations, and a third would be a state nobody has seen.
#[must_use]
pub const fn listing_state_from_status(status: &str) -> Option<ListingState> {
    match status.as_bytes() {
        b"ACTIVE" => Some(ListingState::Live),
        b"NOT_ACTIVE" => Some(ListingState::Draft),
        _ => None,
    }
}

/// The write-evidence body digest, over the raw response bytes. Every
/// evidence-producing cell states one: an empty body digests to a stable
/// value, which is a fact, where `None` would say the adapter could not state
/// one. Byte-identical to the Tes adapter's, because the digest is the
/// evidence contract rather than a per-adapter convention.
fn body_digest(bytes: &[u8]) -> ContentHash {
    ContentHash(Sha256::digest(bytes).into())
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

/// The invariant part of one multipart upload. The date is not among them:
/// each signed call reads the clock for itself, so the upload carries only
/// the attempt's own instant to derive each read's `requestTime` from.
struct S3Upload<'a> {
    ticket: &'a UploadTicket,
    key_id: &'a AwsKeyId,
    content_type: &'a str,
    part_size: usize,
    now: Timestamp,
}

/// One signed S3 call within an upload, and the ordinal that keeps its clock
/// read distinct from its siblings'.
struct S3Step<'a> {
    operation: &'a S3Operation,
    content_md5: Option<&'a str>,
    ordinal: u32,
}

/// The initiate leads, part `n` takes ordinal `n` (S3 numbers parts from one)
/// and the completion follows the last part.
const INITIATE_STEP: u32 = 0;

const fn complete_step(parts: u32) -> u32 {
    parts.saturating_add(1)
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
        form: FormId,
    ) -> Result<FormSchemaFingerprint, AdapterError> {
        let written = write_model::written_field_paths();
        // A scrape failure during the preflight is drift, and only the drift
        // report reaches the machine's drift path: the driver routes every
        // other preflight error as a transient and abandons the run, so a
        // render that stopped carrying an anchor would be retried hourly
        // instead of halting the inventory and capturing diagnostics.
        let page = self
            .form_page(FormTarget::CreateDigital)
            .await
            .map_err(|error| Self::preflight_drift(error, form, &written))?;
        let declared = page.tokens().unlocked_field_names();
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

    /// The create chain, in the order the capture records it: render the
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
            .send_ambiguous_on_loss(endpoints::submit_form_request(target, body))
            .await?;
        let landing = classify_submit(&response)?;
        Ok(SubmitEvidence {
            http_status: Some(response.status),
            response_body_digest: Some(body_digest(&response.body)),
            landed_on_route: Some(landing.location.clone()),
            landed: Some(RemoteListingId::Tpt {
                product_id: landing.product.0,
            }),
            observed_lag: false,
        })
    }

    /// `_from` is genuinely unused: the edit form is a full replace whichever
    /// side of the draft line the product is on, so only `to` selects
    /// anything. `now` is unused because the edit body carries no instant.
    ///
    /// The edit is a full replace and does not preserve `PAGES`,
    /// `COMMON_CORE_ID`, `COUNTRY_ID_FLAG`, `DURATION`, `ANSWER_KEY` or
    /// `TAX_CODE_ID`: `edit_fields` posts each of them empty or absent and
    /// `listing_from_field_set` hardcodes `tax_code: None`. That is safe only
    /// while every bound mapping was bound by this system's own create, which
    /// posts none of the six either — so the blanking is a no-op on every
    /// listing that can reach here today, pinned by
    /// `an_edit_reposts_exactly_what_a_create_posts_for_the_fields_it_does_not_carry`.
    /// Adopting a listing this system did not create makes the blanking live
    /// and is Phase 4's to answer.
    async fn revise(
        &self,
        plan: RevisePlan,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        let product = product_from_locator(&ListingLocator::Durable(plan.subject))?;
        let status = match plan.transition.to {
            ListingState::Draft => StatusUser::Draft,
            ListingState::Live => StatusUser::Live,
        };
        let (landing, response) = self.post_edit(product, &plan.fields, status).await?;
        Ok(SubmitEvidence {
            http_status: Some(response.status),
            response_body_digest: Some(body_digest(&response.body)),
            landed_on_route: Some(landing.location.clone()),
            landed: Some(RemoteListingId::Tpt {
                product_id: landing.product.0,
            }),
            observed_lag: false,
        })
    }

    /// `plan.state` is unused: `RemoveResource` takes a product id and
    /// nothing else, so TPT has no per-state removal route to choose between.
    ///
    /// `http_status` is `None` rather than invented: the mutation's transport
    /// status is consumed by the GraphQL classifier, so this cell has none of
    /// its own to state.
    async fn remove(
        &self,
        plan: RemovalPlan,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        let product = product_from_locator(&ListingLocator::Durable(plan.subject))?;
        let route = endpoints::remove_resource_request(product).url;
        let (deleted, body) = self.post_remove(product).await?;
        Ok(SubmitEvidence {
            http_status: None,
            response_body_digest: Some(body_digest(&body)),
            landed_on_route: Some(route),
            landed: Some(RemoteListingId::Tpt {
                product_id: deleted.0,
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
        // A catalogue page that parsed and does not carry the product is an
        // observation that the product is not there, not an indeterminate
        // read: `MyProductListings` is the only witness TPT offers, and the
        // walk that produced this page answered for the seller's whole
        // catalogue. Answering `Ambiguous` here halted the tenant's inventory
        // on the read-back that immediately follows every create.
        let Some(entry) = page.entries.into_iter().find(|entry| entry.id == product) else {
            return Ok(ObservedListing {
                id: RemoteListingId::Tpt {
                    product_id: product.0,
                },
                fields: vec![],
                lifecycle: RemoteLifecycle::Absent,
            });
        };
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
        reason: &FetchReason,
        id: ProductId,
    ) -> impl core::future::Future<Output = Result<Vec<u8>, AdapterError>> + Send {
        Self::download_resource_bundle(self, reason, id)
    }

    fn fetch_for_import(
        &self,
        reason: &FetchReason,
        id: ProductId,
    ) -> impl core::future::Future<Output = Result<ImportedListing, AdapterError>> + Send {
        Self::fetch_for_import(self, reason, id)
    }
}
