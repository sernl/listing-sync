//! The adapter flows over the M0-confirmed endpoints: create-and-populate,
//! publish, delete with positive verification, and the fetch-reason-gated
//! read. Every step classifies its own response; no step trusts a status
//! another step produced.
//!
//! The `FieldSet` contract is this crate's on both sides: `project_fields`
//! renders it from a projected listing and the submit parses it back.
//! `Title` is plain text, `Description` is markdown, `Price` is a licence
//! token — a free-tier `CC-BY`, `CC-BY-SA` or `CC-BY-ND`, or `TES-PAID:<minor
//! units>` — `Taxonomy` is JSON `{"categories": [..], "mainType": n}`, and
//! `Grades` is JSON `{"ageRanges": [..], "ages": [..], "mainAge": n}`.

use serde_json::Value;
use sha2::{Digest, Sha256};
use tam_marketplace::transport::{HttpResponse, ResponseHeader, Transport};
use tam_marketplace::{
    AdapterError, AmbiguityCause, FetchReason, FieldSet, FileContent, FileSource, FirstPartyExport,
    FormId, FormSchemaFingerprint, IdempotencyKey, ImportedListing, ListingLocator, ListingState,
    MarketplaceAdapter, NativeAxis, ObservedListing, ProjectedListing, RemoteLifecycle,
    RemoteListingId, RemovalPlan, RevisePlan, SubmitEvidence,
};
use tam_types::{
    ContentHash, CopyFormat, CurrencyRule, FailureCode, FailureDetail, FieldKey, ImportedPrice,
    ImportedTerm, InventoryId, Money, PriceIntent, TermKind, Timestamp,
};

use crate::classify::{
    classify_create, classify_delete_status, classify_read, classify_read_bytes,
    classify_transport, classify_write, classify_write_json, classify_write_status,
};
use crate::endpoints::{
    self, CatalogueEntry, DraftId, FreeLicence, PresignedUpload, TesAges, TesLicence, TesListing,
    TesPrice, TesPricing,
};
use crate::schema;

/// The two Tes inventories share this one adapter type; everything that
/// differs between GB and US is data, per the design's crate table.
pub struct TesAdapter<T, F> {
    inventory: InventoryId,
    transport: T,
    file_source: F,
}

/// The read that witnesses whether this state's resource is still there.
///
/// A free function rather than an inherent method: [`ListingState`] is the
/// seam's type, and an inherent impl on it here is E0116.
fn read_request(state: ListingState, id: DraftId) -> tam_marketplace::transport::HttpRequest {
    match state {
        ListingState::Draft => endpoints::read_draft_request(id),
        ListingState::Live => endpoints::read_resource_request(id),
    }
}

/// The delete that removes it. `DELETE .../{id}/draft` removes only the
/// draft overlay and `DELETE .../{id}` 404s for a draft-only resource, so
/// neither stands in for the other.
fn delete_request(state: ListingState, id: DraftId) -> tam_marketplace::transport::HttpRequest {
    match state {
        ListingState::Draft => endpoints::delete_draft_request(id),
        ListingState::Live => endpoints::delete_resource_request(id),
    }
}

/// What one lifecycle write observed about itself. Every cell posts and
/// classifies and none of them verifies, so `observed_lag` is false
/// throughout: the lag is the driver's to observe, and no cell polls inside
/// itself. An empty body digests to a stable value, which is a fact, where
/// `None` would say the adapter could not state one.
fn write_evidence(response: &HttpResponse, route: String, id: DraftId) -> SubmitEvidence {
    let digest: [u8; 32] = Sha256::digest(&response.body).into();
    SubmitEvidence {
        http_status: Some(response.status),
        response_body_digest: Some(ContentHash(digest)),
        landed_on_route: Some(route),
        landed: Some(RemoteListingId::Tes {
            url: id.canonical_url(),
        }),
        observed_lag: false,
    }
}

/// What Tes's own routes call the thing this state addresses. Error-message
/// text, so it stays in Tes's vocabulary rather than moving with the variant
/// name.
#[must_use]
pub const fn route_name(state: ListingState) -> &'static str {
    match state {
        ListingState::Draft => "draft",
        ListingState::Live => "published resource",
    }
}

/// What a bundle starts with.
///
/// Four bytes rather than a dependency on the ingest pipeline: this crate
/// asks only whether the signed url answered a file or a page, and adding an
/// edge to `tam-pipeline` to learn that would be a dependency decision for a
/// question this size.
const BUNDLE_MAGIC: &[u8] = b"PK\x03\x04";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotATesInventory(pub InventoryId);

impl core::fmt::Display for NotATesInventory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?} is not a Tes inventory", self.0)
    }
}

impl core::error::Error for NotATesInventory {}

impl<T: Transport, F: FileSource> TesAdapter<T, F> {
    pub fn new(
        inventory: InventoryId,
        transport: T,
        file_source: F,
    ) -> Result<Self, NotATesInventory> {
        if inventory.marketplace() != tam_types::Marketplace::Tes {
            return Err(NotATesInventory(inventory));
        }
        Ok(Self {
            inventory,
            transport,
            file_source,
        })
    }

    async fn send(
        &self,
        request: tam_marketplace::transport::HttpRequest,
    ) -> Result<tam_marketplace::transport::HttpResponse, AdapterError> {
        self.transport
            .send(request)
            .await
            .map_err(classify_transport)
    }

    /// Creates a draft, sets its metadata and uploads every file. The draft
    /// is a first-class non-live state on Tes, so this whole flow is safe to
    /// run without publishing anything.
    pub async fn create_listing(
        &self,
        listing: &TesListing,
        files: &[FileContent],
    ) -> Result<DraftId, AdapterError> {
        let created = self.send(endpoints::create_draft_request()).await?;
        let id = classify_create(&created)?;

        let metadata = self
            .send(endpoints::set_metadata_request(id, listing))
            .await?;
        classify_write(&metadata, id.0)?;

        for (index, file) in files.iter().enumerate() {
            self.upload_file(id, index, file).await?;
        }
        Ok(id)
    }

    /// The transport, so a harness can interrogate its cassette.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// One file's presign, S3 POST and confirm handshake. Public because it
    /// is a meaningful unit of work on its own and the confirm trap deserves
    /// a direct test.
    pub async fn upload_file(
        &self,
        id: DraftId,
        index: usize,
        file: &FileContent,
    ) -> Result<(), AdapterError> {
        let temp_id = format!("TEMP-{index}");
        let presign = self
            .send(endpoints::presign_request(id, &file.file_name, &temp_id))
            .await?;
        let presign_body = classify_write_json(&presign)?;
        let upload: PresignedUpload =
            endpoints::parse_presign(&presign_body, &file.file_name, &file.content_type).map_err(
                |error| AdapterError::Rejected {
                    code: FailureCode::UploadRejected,
                    detail: FailureDetail(error.to_string()),
                },
            )?;

        let s3 = self
            .send(endpoints::s3_upload_request(
                &upload,
                tam_marketplace::transport::FilePart {
                    part_name: "file".to_owned(),
                    file_name: file.file_name.clone(),
                    content_type: file.content_type.clone(),
                    bytes: file.bytes.clone(),
                },
            ))
            .await?;
        classify_write_status(&s3)?;

        let confirmed = self.send(endpoints::confirm_request(id, &upload)).await?;
        let confirm_body = classify_write_json(&confirmed)?;
        let uploaded = confirm_body
            .get(0)
            .and_then(|attachment| attachment.get("isUploaded"))
            .and_then(Value::as_bool)
            == Some(true);
        if !uploaded {
            // The M0 trap: an echo missing the s3pending key leaves
            // isUploaded false while every status on the way said 2xx.
            return Err(AdapterError::Rejected {
                code: FailureCode::SubmitNoConfirmation,
                detail: FailureDetail("confirm left isUploaded false".to_owned()),
            });
        }
        Ok(())
    }

    /// Rewrites an existing draft's metadata. Tes has no partial edit: the
    /// draft POST is a full restatement, so the caller hands over the listing
    /// it wants the draft to carry, exactly as the create does. The response
    /// must be JSON naming this draft, which is the same positive assertion
    /// the create's own metadata step makes.
    pub async fn update(&self, id: DraftId, listing: &TesListing) -> Result<(), AdapterError> {
        self.post_metadata(id, listing).await.map(drop)
    }

    /// The metadata POST and its own classification, without any read after
    /// it. The trait's `revise` takes this half and leaves the verification
    /// to the driver, which is the only layer holding a budget to poll with.
    async fn post_metadata(
        &self,
        id: DraftId,
        listing: &TesListing,
    ) -> Result<HttpResponse, AdapterError> {
        let written = self
            .send(endpoints::set_metadata_request(id, listing))
            .await?;
        classify_write(&written, id.0)?;
        Ok(written)
    }

    /// The publish POST and its status classification, without the state read
    /// that [`Self::publish`] proves itself with.
    async fn post_publish(
        &self,
        id: DraftId,
        listing: &TesListing,
    ) -> Result<HttpResponse, AdapterError> {
        let published = self.send(endpoints::publish_request(id, listing)).await?;
        classify_write_status(&published)?;
        Ok(published)
    }

    /// The delete POST and nothing else — not even a status classification,
    /// because `DELETE .../{id}/draft` answers 204 whether or not it removed
    /// anything (the misleading-204 measured in M0), so only a read can
    /// settle a deletion. [`Self::delete`] follows this with that read; the
    /// trait's `remove` leaves it to the driver's poll, because the route
    /// that would answer lags by seconds.
    async fn post_delete(
        &self,
        id: DraftId,
        state: ListingState,
    ) -> Result<HttpResponse, AdapterError> {
        self.send(delete_request(state, id)).await
    }

    /// Publishes the draft, then proves it by reading the state back. The
    /// publish POST's own 2xx is never the verdict.
    ///
    /// The listing travels because the publish endpoint re-posts the whole of
    /// it: the licence and, for a paid listing, the price both reach the API
    /// here rather than on the draft alone.
    pub async fn publish(&self, id: DraftId, listing: &TesListing) -> Result<Value, AdapterError> {
        self.post_publish(id, listing).await?;
        let state = self.resource_state(id).await?;
        let is_published = state.get("draft").and_then(Value::as_bool) == Some(false)
            || state.get("isPublic").and_then(Value::as_bool) == Some(true);
        if is_published {
            Ok(state)
        } else {
            Err(AdapterError::Ambiguous(
                AmbiguityCause::ReadBackIndeterminate,
            ))
        }
    }

    /// Deletes a listing in the state the caller states it is in, and reports
    /// success only after the route that state lives at returns 404.
    ///
    /// The state is an argument rather than something this method discovers,
    /// because discovering it is unsound: `/resources/{id}` lags a publish by
    /// seconds — measured live 2026-08-29, a resource whose publish had
    /// already been confirmed still answered 404 there — so a probe run soon
    /// after a write reads a live listing as a draft and deletes only its
    /// overlay, leaving the live listing standing. That is exactly what
    /// happened, and a caller that just wrote always knows what it wrote.
    ///
    /// Each route answers the other's resource with a status that reads like
    /// success, which is why the verdict is the follow-up read on the stated
    /// state's own route and never the delete's own status.
    pub async fn delete(&self, id: DraftId, state: ListingState) -> Result<(), AdapterError> {
        self.post_delete(id, state).await?;
        let after = self.send(read_request(state, id)).await?;
        gone(after.status, route_name(state))
    }

    /// Deletes a never-published draft. `DELETE /resources/{id}` 404s for a
    /// draft-only resource without removing it, and a resource read 404s for a
    /// draft whether or not it was deleted, so both the deletion and its proof
    /// run on the `/draft` route the draft actually lives at.
    pub async fn delete_draft(&self, id: DraftId) -> Result<(), AdapterError> {
        self.delete(id, ListingState::Draft).await
    }

    /// Deletes a published resource. The mirror of [`Self::delete_draft`]:
    /// `/resources/{id}` is the buyer-facing route, the one the delete has to
    /// stop answering, and the one the live run confirmed goes to 404 first —
    /// the dashboard catalogue kept listing a resource this route had already
    /// stopped serving, so the catalogue cannot witness a deletion.
    pub async fn delete_published(&self, id: DraftId) -> Result<(), AdapterError> {
        self.delete(id, ListingState::Live).await
    }

    /// Whether the route a state lives at still answers for this id — the one
    /// read that can witness a deletion, and the one a caller polls when the
    /// API has not caught up with its own write.
    ///
    /// Each route answers only for its own state, so this is not a way to
    /// discover which state a listing is in: a published resource 404s here
    /// under [`ListingState::Live`] for the seconds after its publish,
    /// and a draft 404s under it forever.
    pub async fn is_present(&self, id: DraftId, state: ListingState) -> Result<bool, AdapterError> {
        let read = self.send(read_request(state, id)).await?;
        if read.status == 404 {
            return Ok(false);
        }
        classify_read(&read)?;
        Ok(true)
    }

    /// Deletes a listing whose state the caller genuinely does not know,
    /// recovering it from the route the resource answers on: a never-published
    /// draft 404s on `/resources/{id}` and a published resource does not.
    ///
    /// A last resort, and never the path for a caller that just wrote. The
    /// probe reads the lagging route described on [`Self::delete`], so soon
    /// after a publish it answers "draft" for a live listing and this deletes
    /// the wrong thing. Reconciliation, which meets listings it did not
    /// create, is what this exists for.
    ///
    /// Anything that is neither answer settles as the read classifier's own
    /// verdict — a lapsed session and a rate limit are named conditions, not a
    /// resource that happens to be absent.
    pub async fn delete_probing_state(&self, id: DraftId) -> Result<(), AdapterError> {
        let probe = self.send(endpoints::read_resource_request(id)).await?;
        let state = if probe.status == 404 {
            ListingState::Draft
        } else {
            classify_read(&probe)?;
            ListingState::Live
        };
        self.delete(id, state).await
    }

    /// A tier-two read internal to a write flow: one fetch of one listing by
    /// an identifier this flow already holds, immediately following its own
    /// authorised write. The public read path is [`MarketplaceAdapter::read_back`],
    /// which demands a `FetchReason`.
    async fn resource_state(&self, id: DraftId) -> Result<Value, AdapterError> {
        let draft = self.send(endpoints::read_draft_request(id)).await?;
        if draft.status == 200 {
            if let Ok(value) = classify_read(&draft) {
                if value.get("id").is_some() {
                    return Ok(value);
                }
            }
        }
        let resource = self.send(endpoints::read_resource_request(id)).await?;
        let value = classify_read(&resource)?;
        if value.get("id").is_some() {
            Ok(value)
        } else {
            Err(AdapterError::Ambiguous(
                AmbiguityCause::ReadBackIndeterminate,
            ))
        }
    }

    fn draft_id_from_locator(locator: &ListingLocator) -> Result<DraftId, AdapterError> {
        match locator {
            ListingLocator::Durable(RemoteListingId::Tes { url }) => url
                .rsplit(['-', '/'])
                .next()
                .and_then(|tail| tail.parse::<i64>().ok())
                .map(DraftId)
                .ok_or_else(|| AdapterError::Rejected {
                    code: FailureCode::Other,
                    detail: FailureDetail(format!(
                        "no numeric id recoverable from the Tes url {url:?}"
                    )),
                }),
            ListingLocator::Durable(_) => Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail("a non-Tes identifier reached the Tes adapter".to_owned()),
            }),
            // A recorded title is an identification, never an address. The
            // reconcile carries one only as far as the catalogue search; what
            // that search finds is a durable id, and it is the durable id the
            // verifying read-back addresses.
            ListingLocator::Recorded { .. } => Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail(
                    "a recorded title names no listing to read; the search resolves it to a \
                     durable identifier first"
                        .to_owned(),
                ),
            }),
            // Marker search is the reconciliation flow's tool and lands with
            // the job engine (M1d).
            ListingLocator::Marker { .. } => Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail("marker reconciliation is not implemented yet".to_owned()),
            }),
        }
    }

    fn listing_from_field_set(
        inventory: InventoryId,
        fields: &FieldSet,
    ) -> Result<TesListing, AdapterError> {
        let entry = |key: FieldKey| -> Result<&str, AdapterError> {
            fields
                .entries
                .iter()
                .find(|(field, _)| *field == key)
                .map(|(_, value)| value.as_str())
                .ok_or_else(|| AdapterError::Rejected {
                    code: FailureCode::UploadRejected,
                    detail: FailureDetail(format!("the projection omitted {key:?}")),
                })
        };
        let pricing = pricing_from_token(entry(FieldKey::Price)?)?;
        let taxonomy: Value =
            serde_json::from_str(entry(FieldKey::Taxonomy)?).map_err(|error| {
                AdapterError::Rejected {
                    code: FailureCode::UploadRejected,
                    detail: FailureDetail(format!("taxonomy entry is not JSON: {error}")),
                }
            })?;
        let grades: Value = serde_json::from_str(entry(FieldKey::Grades)?).map_err(|error| {
            AdapterError::Rejected {
                code: FailureCode::UploadRejected,
                detail: FailureDetail(format!("grades entry is not JSON: {error}")),
            }
        })?;
        let ids = |value: &Value, key: &str| -> Vec<i64> {
            value
                .get(key)
                .and_then(Value::as_array)
                .map(|array| array.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default()
        };
        Ok(TesListing {
            title: entry(FieldKey::Title)?.to_owned(),
            description_raw: entry(FieldKey::Description)?.to_owned(),
            // The declaration travels with the bytes or the write refuses:
            // posting one format's markup under the other's type is what the
            // seam carries this field to make impossible.
            description_format: fields.body_format.ok_or_else(|| AdapterError::Rejected {
                code: FailureCode::UploadRejected,
                detail: FailureDetail(
                    "the projection carried a description and no format for it".to_owned(),
                ),
            })?,
            category_ids: ids(&taxonomy, "categories"),
            // The channel is the inventory's, and the seam's own JSON names
            // it, so a read-back parses the field the write emitted rather
            // than the one this country does not use.
            age_channel: TesAges::of(inventory, ids(&grades, TesAges::empty(inventory).field())),
            ages: ids(&grades, "ages"),
            main_type: taxonomy.get("mainType").and_then(Value::as_i64),
            main_age: grades.get("mainAge").and_then(Value::as_i64),
            pricing,
        })
    }
}

impl<T: Transport, F: FileSource> MarketplaceAdapter for TesAdapter<T, F> {
    fn inventory(&self) -> InventoryId {
        self.inventory
    }

    /// Tes's own wire shape, rendered here rather than in the engine that
    /// seeds the item: a licence token in `Price` — carrying the price too
    /// where the licence is the paid one — the numeric category ids in
    /// `Taxonomy`, and the age ranges with the derived age list in `Grades`.
    /// The submit's own `listing_from_field_set` parses exactly this back, so
    /// both halves of the contract sit in one file.
    ///
    /// A category the crosswalk left without a numeric id is refused: Tes
    /// addresses categories by number and there is nothing to send.
    ///
    /// The body is not. Tes takes either format and posts the matching
    /// `descriptionRawType`, which the 2026-08-29 probe established against a
    /// live draft, so a TPT-sourced HTML body crosses as itself rather than
    /// being refused or converted. What the declaration still prevents is
    /// posting one format's bytes under the other's type; carrying it through
    /// the field set is what makes that unrepresentable rather than merely
    /// avoided.
    fn project_fields(&self, listing: &ProjectedListing) -> Result<FieldSet, AdapterError> {
        let elected = licence_from(&listing.natives);
        let licence = match listing.price {
            PriceIntent::Free => elected
                .ok_or_else(|| {
                    unprojectable_licence(
                        "Tes requires a licence and the projection carried none; the seller's \
                         election is the only source and this adapter will not choose a rights \
                         grant on their behalf"
                            .to_owned(),
                    )
                })?
                .to_owned(),
            PriceIntent::Paid(money) => paid_price_token(self.inventory, elected, money)?,
        };
        let mut categories: Vec<i64> = Vec::new();
        for term in &listing.taxonomy {
            let native = term.native_id.as_deref().ok_or_else(|| {
                unprojectable_category("a Tes taxonomy path must carry its numeric id".to_owned())
            })?;
            categories.push(native.parse().map_err(|_| {
                unprojectable_category(format!("non-numeric Tes category id {native:?}"))
            })?);
        }
        // Refused rather than skipped, exactly as the category loop above
        // does. A grade the crosswalk left without a numeric id used to be
        // dropped here, which published a listing carrying fewer grades than
        // the seller authored and said nothing about it.
        let mut grade_ids: Vec<i64> = Vec::new();
        for term in &listing.grades {
            let native = term.native_id.as_deref().ok_or_else(|| {
                unprojectable_grade("a Tes grade path must carry its numeric id".to_owned())
            })?;
            grade_ids.push(native.parse().map_err(|_| {
                unprojectable_grade(format!("non-numeric Tes grade id {native:?}"))
            })?);
        }
        // The resource type is projected or it is absent. Sending zero named
        // a real Tes type on every listing we ever created; the registry says
        // the field is optional, so an unprojected axis omits the key.
        let main_type =
            match native_of(&listing.natives, TermKind::ResourceType) {
                Some(native) => Some(native.parse::<i64>().map_err(|_| {
                    refused(format!("non-numeric Tes resource type id {native:?}"))
                })?),
                None => None,
            };
        let age_channel = TesAges::of(self.inventory, grade_ids);
        // P.1, settled by the 2026-08-29 capture. The wire's age fields are
        // derived from the bands the seller declared, not from the canonical
        // interval: `ages` is the union of those bands' own age sets and
        // `mainAge` names a band. The interval is the wrong source twice
        // over -- it fills the gap between disjoint bands, and its low year
        // reaches `mainAge` as a band id, where 7 names "Age not applicable"
        // and 11 names nothing at all.
        let endpoints::DerivedAges { ages, main_age } =
            age_channel
                .derived_ages()
                .map_err(|endpoints::UnknownAgeBand(id)| {
                    unprojectable_grade(format!(
                        "Tes grade id {id} is not one of the seven age bands the uploader offers"
                    ))
                })?;
        Ok(FieldSet {
            body_format: Some(listing.body_format),
            entries: vec![
                (FieldKey::Title, listing.title.clone()),
                (FieldKey::Description, listing.body.clone()),
                (FieldKey::Price, licence),
                (FieldKey::Taxonomy, taxonomy_entry(&categories, main_type)),
                (
                    FieldKey::Grades,
                    grades_entry(&age_channel, &ages, main_age),
                ),
            ],
            files: listing.files.clone(),
            appropriate_for_country: None,
        })
    }

    /// The pre-flight is the M0 loop as a probe: create a draft, write every
    /// field this adapter writes, read the result back, fingerprint it, and
    /// delete the probe. The write comes before the assertion because the
    /// live API omits null scalar keys from a draft's JSON, so an empty
    /// draft cannot witness the written-field set.
    async fn assert_form_schema(
        &self,
        form: FormId,
    ) -> Result<FormSchemaFingerprint, AdapterError> {
        let created = self.send(endpoints::create_draft_request()).await?;
        let id = classify_create(&created)?;
        let written = self
            .send(endpoints::set_metadata_request(
                id,
                &endpoints::probe_listing(),
            ))
            .await
            .and_then(|response| classify_write_json(&response).map(drop));
        let state = match written {
            Ok(()) => self.resource_state(id).await,
            Err(error) => Err(error),
        };
        let deleted = self.delete_draft(id).await;
        let observed = schema::field_names(&state?);
        deleted?;
        let missing = schema::missing_written_fields(&observed);
        if missing.is_empty() {
            Ok(schema::fingerprint(&observed))
        } else {
            Err(AdapterError::SchemaDrift(Box::new(
                tam_marketplace::SchemaDrift {
                    form,
                    expected: schema::fingerprint(&schema::WRITTEN_DRAFT_FIELDS.map(str::to_owned)),
                    observed: schema::fingerprint(&observed),
                    added: Vec::new(),
                    removed: missing.into_iter().map(str::to_owned).collect(),
                },
            )))
        }
    }

    /// `IdempotencyKey` is unused here because Tes offers no idempotent
    /// create; the job ledger's fencing (`write_attempt_one_in_flight`) is
    /// what stands in for it, which is why the key still must exist at the
    /// call site. `now` is unused because no Tes request carries an instant:
    /// the JSON API stamps its own.
    async fn submit(
        &self,
        _key: IdempotencyKey,
        fields: FieldSet,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        let listing = Self::listing_from_field_set(self.inventory, &fields)?;
        let mut contents = Vec::with_capacity(fields.files.len());
        for file in &fields.files {
            let content =
                self.file_source
                    .fetch(*file)
                    .await
                    .map_err(|error| AdapterError::Rejected {
                        code: FailureCode::UploadRejected,
                        detail: FailureDetail(format!("file source: {error:?}")),
                    })?;
            contents.push(content);
        }
        let id = self.create_listing(&listing, &contents).await?;
        let state = self.resource_state(id).await?;
        let digest: [u8; 32] = Sha256::digest(state.to_string().as_bytes()).into();
        Ok(SubmitEvidence {
            http_status: Some(200),
            response_body_digest: Some(ContentHash(digest)),
            landed_on_route: Some(id.canonical_url()),
            // The durable identifier the create returned, so the pre-settle
            // verification read addresses the resource directly rather than
            // search for a marker the JSON API never carried.
            landed: Some(RemoteListingId::Tes {
                url: id.canonical_url(),
            }),
            observed_lag: false,
        })
    }

    /// `now` is unused for the same reason it is unused on
    /// [`MarketplaceAdapter::submit`]: no Tes request carries an instant.
    ///
    /// Only the two transitions out of `Draft` are captured. No capture shows
    /// `POST /{id}/draft` against a published resource, and M0 recorded that
    /// the `/draft` route is a draft *overlay* whose delete answers a
    /// misleading 204 — so guessing that it edits or unpublishes a live
    /// listing would edit an overlay and leave the live listing standing.
    async fn revise(
        &self,
        plan: RevisePlan,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        let id = Self::draft_id_from_locator(&ListingLocator::Durable(plan.subject))?;
        let listing = Self::listing_from_field_set(self.inventory, &plan.fields)?;
        match (plan.transition.from, plan.transition.to) {
            // The metadata POST answers with JSON naming the draft, and
            // `post_metadata` asserts it: free evidence the write's own
            // response carries, where the publish route answers status only.
            (ListingState::Draft, ListingState::Draft) => {
                let response = self.post_metadata(id, &listing).await?;
                let route = format!("{}/api/v2/resources/{}/draft", endpoints::ORIGIN, id.0);
                Ok(write_evidence(&response, route, id))
            }
            (ListingState::Draft, ListingState::Live) => {
                let response = self.post_publish(id, &listing).await?;
                let route = format!(
                    "{}/api/v2/resources/{}/draft/publish",
                    endpoints::ORIGIN,
                    id.0
                );
                Ok(write_evidence(&response, route, id))
            }
            (ListingState::Live, ListingState::Live) => Err(AdapterError::Uncaptured {
                capability: "tes.edit_published",
            }),
            (ListingState::Live, ListingState::Draft) => Err(AdapterError::Uncaptured {
                capability: "tes.unpublish",
            }),
        }
    }

    /// The state selects which delete is posted and that is load-bearing:
    /// each route 404s for the other's resource, and choosing it from a probe
    /// is the 2026-08-29 incident, where a publish-lagged read said "draft"
    /// for a live listing and only its overlay was removed.
    ///
    /// The delete's own status settles nothing, which is why this classifies
    /// with [`classify_delete_status`] rather than the write classifier: the
    /// 404 an already-gone listing answers is the same 404 the wrong route
    /// answers, and only the driver's absence poll can tell them apart.
    ///
    /// `plan.attempt` is unused here: the authorisation is structural, in the
    /// closed `Effect` set that is the only thing able to reach this call.
    async fn remove(
        &self,
        plan: RemovalPlan,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        let id = Self::draft_id_from_locator(&ListingLocator::Durable(plan.subject))?;
        let response = self.post_delete(id, plan.state).await?;
        classify_delete_status(&response)?;
        let route = match plan.state {
            ListingState::Draft => format!("{}/api/v2/resources/{}/draft", endpoints::ORIGIN, id.0),
            ListingState::Live => id.canonical_url(),
        };
        Ok(write_evidence(&response, route, id))
    }

    async fn read_back(
        &self,
        locator: ListingLocator,
        _reason: FetchReason,
        observed_at: Timestamp,
    ) -> Result<ObservedListing, AdapterError> {
        let id = Self::draft_id_from_locator(&locator)?;
        let subject = match &locator {
            ListingLocator::Durable(durable) => durable.clone(),
            // Unreachable for `Recorded`, which `draft_id_from_locator` has
            // already refused above; both non-durable arms name the resource
            // the recovered id names.
            ListingLocator::Marker { .. } | ListingLocator::Recorded { .. } => id.remote(),
        };
        let state = match self.resource_state(id).await {
            Ok(state) => state,
            // Neither route answered, which is a fact about the listing and
            // not a failure of the read: `resource_state` reads `/{id}/draft`
            // first and falls through to `/resources/{id}`, so its 404 means
            // both are silent — the same union the live runner proves a
            // deletion with. The catch is local to this method on purpose.
            // `resource_state`'s four other callers — `publish`, which needs
            // the JSON to evaluate its published test, `assert_form_schema`,
            // which reads a 404 as the probe draft having vanished, `submit`,
            // which reads one as an unreadable create, and `fetch_for_import`
            // — all still need the 404 to stay an error.
            Err(AdapterError::Rejected {
                code: FailureCode::PreconditionElementAbsent,
                ..
            }) => {
                return Ok(ObservedListing {
                    id: subject,
                    fields: vec![],
                    lifecycle: RemoteLifecycle::Absent,
                })
            }
            Err(error) => return Err(error),
        };
        let mut fields = Vec::new();
        if let Some(title) = state.get("title").and_then(Value::as_str) {
            fields.push((FieldKey::Title, title.to_owned()));
        }
        if let Some(description) = state
            .get("descriptionRaw")
            .or_else(|| state.get("description"))
            .and_then(Value::as_str)
        {
            fields.push((FieldKey::Description, description.to_owned()));
        }
        let lifecycle = if state.get("draft").and_then(Value::as_bool) == Some(true) {
            RemoteLifecycle::Draft
        } else {
            // The earliest instant this reader can attest to; the true
            // publication instant is the marketplace's own.
            RemoteLifecycle::Live { since: observed_at }
        };
        Ok(ObservedListing {
            id: subject,
            fields,
            lifecycle,
        })
    }
}

/// How many pages a catalogue walk will request before it refuses to
/// continue. The walk ends at the first empty page; this cap only bounds a
/// walk whose end never arrives.
pub const CATALOGUE_PAGE_MAX: u32 = 40;

impl<T: Transport, F: FileSource> TesAdapter<T, F> {
    /// The seller's own catalogue: every published resource, then every
    /// draft. Gated on the first-party-export capability like
    /// [`Self::fetch_for_import`], because an enumeration-shaped read is
    /// exactly the tier-one capability and nothing else justifies one.
    pub async fn list_own_resources(
        &self,
        reason: &FetchReason,
    ) -> Result<Vec<CatalogueEntry>, AdapterError> {
        // Two reasons justify an enumeration, and no third does without a
        // decision. `FirstPartyExport` is the tier-one capability this read
        // was written for. `VerifyAttempt` is the reconciliation of a create
        // whose fate the ledger cannot determine: an enumeration justified by
        // an open fencing row, entered only for the attempt being settled, and
        // answering the same complete-or-refuse contract as the export.
        //
        // The machine already reasons in these terms — `SyncMachine::reconcile`
        // reads back under `VerifyAttempt` for the operations that have a
        // durable subject — so admitting it here makes the create's path
        // consistent with the ones beside it rather than carving an exception.
        //
        // This is the only non-export caller. A third is a decision rather
        // than a precedent.
        if !matches!(
            reason,
            FetchReason::FirstPartyExport { .. } | FetchReason::VerifyAttempt { .. }
        ) {
            return Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail(
                    "a catalogue read is justified by the first-party-export capability or by \
                     the fencing row of the attempt it is reconciling, and by nothing else"
                        .to_owned(),
                ),
            });
        }
        let mut entries = Vec::new();
        self.walk_catalogue(endpoints::list_resources_request, true, &mut entries)
            .await?;
        self.walk_catalogue(endpoints::list_drafts_request, false, &mut entries)
            .await?;
        Ok(entries)
    }

    async fn walk_catalogue(
        &self,
        build: fn(u32, u32) -> tam_marketplace::transport::HttpRequest,
        published: bool,
        entries: &mut Vec<CatalogueEntry>,
    ) -> Result<(), AdapterError> {
        for page in 0..CATALOGUE_PAGE_MAX {
            let response = self
                .send(build(page, endpoints::CATALOGUE_PAGE_LIMIT))
                .await?;
            let body = classify_read(&response)?;
            let rows = endpoints::parse_catalogue_page(&body, published).map_err(|error| {
                AdapterError::Rejected {
                    code: FailureCode::VerificationMismatch,
                    detail: FailureDetail(error.to_string()),
                }
            })?;
            if rows.is_empty() {
                return Ok(());
            }
            entries.extend(rows);
        }
        // Reaching the cap means the catalogue never ended, so what was
        // collected is a truncation; the import must not mistake it for the
        // whole catalogue.
        Err(AdapterError::Rejected {
            code: FailureCode::Other,
            detail: FailureDetail(format!(
                "the catalogue walk hit its {CATALOGUE_PAGE_MAX}-page cap without reaching an \
                 empty page; the result would be truncated"
            )),
        })
    }

    /// The seller's own files for a published resource: the bytes of the
    /// bundle Tes assembles. Two steps — the manifest names the bundle, then
    /// the bundle is fetched — and the second step answers a 302 to a signed
    /// CDN url on another host.
    ///
    /// Whether that hop is followed is the transport's to decide, and the two
    /// this crate ships decide differently. The broker gateway followed it and
    /// returned the ZIP, which is what this comment used to describe as though
    /// it were the only case. The live transport on the seller's own device
    /// does not: its session client stops at a host change so the rule about
    /// which client may carry what survives a destination the marketplace
    /// chose, and the hop comes back here as the 3xx it is. Re-issuing it on
    /// the credential-free client is not built, so over that transport this
    /// refuses by name rather than returning bytes.
    ///
    /// Published-only by construction. A draft has no bundle, and that is
    /// reported as a rejection naming the cause rather than as an ambiguity,
    /// because nothing about it is unknown.
    pub async fn download_resource_bundle(
        &self,
        reason: &FetchReason,
        id: DraftId,
    ) -> Result<Vec<u8>, AdapterError> {
        if !matches!(reason, FetchReason::FirstPartyExport { .. }) {
            return Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail(
                    "a file download is justified only by the first-party-export capability"
                        .to_owned(),
                ),
            });
        }
        let manifest = self.send(endpoints::download_manifest_request(id)).await?;
        // A draft's manifest route redirects to an HTML `?error=notfound`
        // page, so a body that will not parse as a manifest means the
        // resource is unpublished, not that the read failed. That page is a
        // Tes page and carries the word the sign-in sniffer keys on, so the
        // classifier calls it a dead session; the state route, which a dead
        // session cannot read, is what tells the two apart.
        let body = match classify_read(&manifest) {
            Ok(body) => body,
            Err(AdapterError::Rejected { .. }) => return Err(no_published_bundle(id)),
            Err(AdapterError::SessionExpired) => {
                return Err(match self.resource_state(id).await {
                    Ok(state) if state.get("draft").and_then(Value::as_bool) == Some(true) => {
                        no_published_bundle(id)
                    }
                    Ok(_) => AdapterError::Rejected {
                        code: FailureCode::Other,
                        detail: FailureDetail(format!(
                            "the download manifest for resource {} answered a page rather than \
                             a manifest while the resource itself still reads, so the session \
                             stands and there is no bundle behind that page",
                            id.0
                        )),
                    },
                    Err(error) => error,
                });
            }
            Err(error) => return Err(error),
        };
        let path = endpoints::parse_download_manifest(&body, id).map_err(|error| match error {
            endpoints::DownloadManifestError::NoPublishedBundle => no_published_bundle(id),
            endpoints::DownloadManifestError::OffOrigin(_) => AdapterError::Rejected {
                code: FailureCode::UnexpectedOrigin,
                detail: FailureDetail(error.to_string()),
            },
        })?;
        let bundle = self.send(endpoints::download_bundle_request(&path)).await?;
        // The bundle hop answers a redirect to a signed url on a content
        // network, and the session transport declines to follow it, so what
        // arrives here is the 3xx itself. Re-issuing it is deliberate rather
        // than automatic: the destination was named by the marketplace.
        if !(300..400).contains(&bundle.status) {
            return classify_read_bytes(&bundle).map(<[u8]>::to_vec);
        }
        let Some(location) = bundle.header(ResponseHeader::Location) else {
            return Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail(format!(
                    "the bundle for resource {} answered {} with no location to follow",
                    id.0, bundle.status
                )),
            });
        };
        // The destination is the marketplace's choice rather than ours, so its
        // shape is examined before a request goes to it. The transport asserts
        // scheme and origin again on the way out, but only as a `NotSent`,
        // which is the class documented as safe to retry; refused here, a
        // permanent condition is named as one. The private-address rule
        // exists nowhere else at all.
        if let Err(why) = endpoints::check_redirect_target(location) {
            return Err(AdapterError::Rejected {
                code: FailureCode::UnexpectedOrigin,
                detail: FailureDetail(format!(
                    "the bundle for resource {} redirects somewhere this download will not \
                     follow: {why}",
                    id.0
                )),
            });
        }
        let followed = self
            .send(endpoints::redirected_bundle_request(location.to_owned()))
            .await?;
        // One hop, and the second 3xx is where that is enforced. Re-issuing
        // again would be this crate following a chain the marketplace controls,
        // which is the thing the session client's policy exists to stop being
        // automatic.
        if (300..400).contains(&followed.status) {
            return Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail(format!(
                    "the signed url for resource {} redirected again, and one hop is all this \
                     download re-issues",
                    id.0
                )),
            });
        }
        // What comes back must be a bundle, and that is decided before the
        // shared classifier sees it, for two reasons that are both about
        // ending up in the halt-the-tenant arm over a condition we can name.
        //
        // A content network's own 5xx would classify as an ambiguous read,
        // which halts a tenant's inventory and waits for an operator; but this
        // hop wrote nothing and its failure is a fetch we can simply describe,
        // so it is a refusal.
        //
        // And an expired signature answers 200 with a page rather than a file.
        // If that page happens to contain the word the session-expiry sniffer
        // looks for — an error page saying "log in" would — the classifier
        // reports `SessionExpired` and the run parks on a session that is
        // perfectly good. Checking the magic first means the bytes decide what
        // they are before anything reads them for what they might say.
        if !(200..300).contains(&followed.status) {
            return Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail(format!(
                    "the signed url for resource {} answered {}; the hop wrote nothing, so \
                     this is a fetch that failed rather than an outcome nobody can determine",
                    id.0, followed.status
                )),
            });
        }
        if !followed.body.starts_with(BUNDLE_MAGIC) {
            return Err(AdapterError::Rejected {
                code: FailureCode::VerificationMismatch,
                detail: FailureDetail(format!(
                    "the signed url for resource {} answered {} bytes that are not an archive; \
                     a signed url that has expired answers a page rather than a file",
                    id.0,
                    followed.body.len()
                )),
            });
        }
        classify_read_bytes(&followed).map(<[u8]>::to_vec)
    }

    /// The first-party import read: the seller's own listing, verbatim, for
    /// canonicalisation. Refuses any reason but `FirstPartyExport`, because
    /// this is the tier-one capability and nothing else justifies an
    /// enumeration-shaped read.
    pub async fn fetch_for_import(
        &self,
        reason: &FetchReason,
        id: DraftId,
    ) -> Result<ImportedListing, AdapterError> {
        if !matches!(reason, FetchReason::FirstPartyExport { .. }) {
            return Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail(
                    "an import read is justified only by the first-party-export capability"
                        .to_owned(),
                ),
            });
        }
        let state = self.resource_state(id).await?;
        let title = state
            .get("title")
            .and_then(Value::as_str)
            .ok_or(AdapterError::Ambiguous(
                AmbiguityCause::ReadBackIndeterminate,
            ))?
            .to_owned();
        let body = state
            .get("descriptionRaw")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let category_native_ids: Vec<String> = state
            .get("categories")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry.get("id"))
                    .filter_map(native_id_string)
                    .collect()
            })
            .unwrap_or_default();

        let inventory = self.inventory;
        let term = |kind: Option<TermKind>, native: &str| ImportedTerm {
            inventory,
            kind,
            segments: vec![native.to_owned()],
            native_id: Some(native.to_owned()),
        };
        let mut native: Vec<ImportedTerm> = category_native_ids
            .iter()
            .map(|id| term(Some(TermKind::Subject), id))
            .collect();
        // The country fork: the uploader takes `ageRanges` for GB and
        // `yearGroups` everywhere else, so exactly one of the two answers the
        // phase axis on any given resource. The other is not a second phase
        // channel — its ids belong to a different vocabulary and mixing them
        // would ask `derive_interval` to read a year group off the age table
        // — so whatever the unused field carries travels untagged.
        //
        // The registry records the same fork on `equivalence_axes`; this is
        // the adapter's own statement of its wire, restated rather than
        // imported because the pure core depends on this crate and not the
        // other way round.
        let (phase_field, other_field) = match inventory {
            InventoryId::TesGb => ("ageRanges", "yearGroups"),
            InventoryId::TesUs | InventoryId::TesNz | InventoryId::Etsy | InventoryId::Tpt => {
                ("yearGroups", "ageRanges")
            }
        };
        for phase in string_list(state.get(phase_field)) {
            native.push(term(Some(TermKind::Phase), &phase));
        }
        for stray in string_list(state.get(other_field)) {
            native.push(term(None, &stray));
        }
        // The curriculum orientation answers no axis this model types, so it
        // travels untagged rather than being filed under a kind it does not
        // mean.
        for orientation in string_list(state.get("curriculum")) {
            native.push(term(None, &orientation));
        }
        if let Some(main_type) = state.get("mainType").and_then(native_id_string) {
            native.push(term(Some(TermKind::ResourceType), &main_type));
        }

        // Declared by the resource rather than guessed from the bytes. The
        // write posts the type the projection carried, so a read that assumed
        // markdown would relabel every HTML body this adapter now writes; a
        // resource predating the field says nothing and is the markdown every
        // earlier write posted.
        let body_format = match state.get("descriptionRawType").and_then(Value::as_str) {
            Some(token) => endpoints::format_from_raw_type(token).ok_or_else(|| {
                refused(format!(
                    "unrecognised descriptionRawType {token:?}; refusing to guess what the \
                     body's bytes are"
                ))
            })?,
            None => CopyFormat::Markdown,
        };

        let licence = state.get("licence").and_then(Value::as_str);
        let rights = licence.map(|token| term(Some(TermKind::Licence), token));
        let price = self.import_price(licence, state.get("price"))?;

        Ok(ImportedListing {
            remote: RemoteListingId::Tes {
                url: format!("https://www.tes.com/teaching-resource/-{}", id.0),
            },
            title,
            body,
            body_format,
            native,
            rights,
            price,
            // A missing `draft` key reads as live, exactly as `read_back`
            // does with the same key, so the state is read rather than
            // assumed on every Tes resource.
            state: Some(
                if state.get("draft").and_then(Value::as_bool) == Some(true) {
                    ListingState::Draft
                } else {
                    ListingState::Live
                },
            ),
        })
    }

    /// The price as Tes states it, with the currency taken from the
    /// inventory's own rule rather than from the number.
    ///
    /// Tes gates the write on the licence — a Creative Commons value with a
    /// price is refused and a paid one without is too — so the licence is
    /// what says whether the number means anything, and an unrecognised token
    /// refuses rather than being read as free.
    ///
    /// The number is the wire's own integer of minor units, carried without
    /// conversion: the captured publish body reads `"price": 500` for
    /// GBP 5.00, the dashboard rows carry the same integer as `price_pence`,
    /// and the draft route echoes the metadata the write posted in that unit.
    /// This read multiplied it by a hundred until 2026-09-07, so every £5.00
    /// listing was imported as £500.00.
    fn import_price(
        &self,
        licence: Option<&str>,
        price: Option<&Value>,
    ) -> Result<ImportedPrice, AdapterError> {
        let Some(token) = licence else {
            return Ok(ImportedPrice::Free);
        };
        let Some(known) = endpoints::ReadLicence::from_token(token) else {
            return Err(refused(format!(
                "unrecognised licence {token:?}; refusing to guess whether this is paid"
            )));
        };
        if !known.is_paid() {
            return Ok(ImportedPrice::Free);
        }
        let value = price.ok_or_else(|| {
            refused(format!(
                "{token} requires a price and the resource carried none"
            ))
        })?;
        let minor_units = minor_units_of(value).ok_or_else(|| {
            refused(format!(
                "Tes states a price as a whole number of minor units, and {value} is not one"
            ))
        })?;
        Ok(ImportedPrice::Paid {
            minor_units,
            // Every Tes inventory is CurrencyRule::Fixed, so the denomination
            // is a fact of the inventory rather than of the number. The write
            // side already mints from the same rule.
            denomination: match self.inventory.currency_rule() {
                CurrencyRule::Fixed(currency) => currency.code().to_owned(),
                CurrencyRule::SellerScoped | CurrencyRule::Unmeasured => {
                    return Err(refused(
                        "a paid Tes resource needs a fixed currency and this inventory                          declares none"
                            .to_owned(),
                    ))
                }
            },
        })
    }
}

/// The three reads above as the cross-platform capability. The bodies stay
/// inherent so the Tes crate's own tests and the import binary keep calling
/// them directly; the trait is what a platform-agnostic importer binds.
impl<T: Transport, F: FileSource> FirstPartyExport for TesAdapter<T, F> {
    type CatalogueEntry = CatalogueEntry;
    type Resource = DraftId;

    fn list_own_resources(
        &self,
        reason: &FetchReason,
    ) -> impl core::future::Future<Output = Result<Vec<CatalogueEntry>, AdapterError>> + Send {
        Self::list_own_resources(self, reason)
    }

    fn download_resource_bundle(
        &self,
        reason: &FetchReason,
        id: DraftId,
    ) -> impl core::future::Future<Output = Result<Vec<u8>, AdapterError>> + Send {
        Self::download_resource_bundle(self, reason, id)
    }

    fn fetch_for_import(
        &self,
        reason: &FetchReason,
        id: DraftId,
    ) -> impl core::future::Future<Output = Result<ImportedListing, AdapterError>> + Send {
        Self::fetch_for_import(self, reason, id)
    }
}

/// The marketplace serialises ids sometimes as numbers and sometimes as
/// strings; the import keeps them as strings, the edge relation's own form.
/// The wire's price integer. A number with a fractional part is refused
/// rather than rounded, because rounding is how a major-unit amount would
/// pass as a hundredth of itself.
fn minor_units_of(value: &Value) -> Option<i64> {
    if let Some(units) = value.as_i64() {
        return (0..=1_000_000_000).contains(&units).then_some(units);
    }
    let float = value.as_f64()?;
    if float.fract() != 0.0 || !(0.0..=1_000_000_000.0).contains(&float) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "whole and range-checked immediately above; the cast is the conversion"
    )]
    Some(float as i64)
}

fn native_id_string(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => Some(text.clone()),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(entries)) => entries.iter().filter_map(native_id_string).collect(),
        Some(single) => native_id_string(single).into_iter().collect(),
        None => Vec::new(),
    }
}

/// The verdict of a delete, which is the follow-up read's status and never
/// the delete's own: `DELETE` answers 204 on both Tes routes whether or not
/// the resource behind the one it took is gone.
fn gone(status: u16, what: &str) -> Result<(), AdapterError> {
    match status {
        404 => Ok(()),
        200 => Err(AdapterError::Rejected {
            code: FailureCode::VerificationMismatch,
            detail: FailureDetail(format!("the {what} is still readable after delete")),
        }),
        _ => Err(AdapterError::Ambiguous(
            AmbiguityCause::ReadBackIndeterminate,
        )),
    }
}

/// The separator between the paid licence token and its amount in the `Price`
/// entry. Neither half can contain it: the licence tokens are a closed set of
/// hyphenated words and the amount is an integer.
const PRICE_TOKEN_SEPARATOR: char = ':';

/// The `Price` entry for a paid listing: the `TES-PAID` licence and the price
/// in the minor units the wire carries.
///
/// The currency is asserted against the inventory's own rule rather than
/// travelling in the token. Tes denominates by inventory and its wire carries
/// a bare integer, so a price whose currency disagrees with the inventory it
/// is bound for would be posted as an amount in a denomination nobody chose;
/// that is refused here rather than converted, because converting is a
/// mapping decision and this is a rendering.
fn paid_price_token(
    inventory: InventoryId,
    elected: Option<&str>,
    money: Money,
) -> Result<String, AdapterError> {
    // A price states the paid branch on its own, and `TES-PAID` is the only
    // paid token this adapter writes, so an absent election is not a rights
    // grant chosen on the seller's behalf. An election naming anything else
    // is refused rather than overridden: the seller stated a licence and
    // posting a different one is the defect this commit removes.
    if let Some(token) = elected {
        if token != TesLicence::TesPaid.as_str() {
            return Err(unprojectable_licence(format!(
                "the elected licence is {token:?} and this listing carries a price; Tes writes \
                 {} for a paid resource and refuses a Creative Commons value with one",
                TesLicence::TesPaid.as_str()
            )));
        }
    }
    match inventory.currency_rule() {
        CurrencyRule::Fixed(currency) => {
            if currency == money.currency() {
                Ok(format!(
                    "{}{PRICE_TOKEN_SEPARATOR}{}",
                    TesLicence::TesPaid.as_str(),
                    money.minor_units()
                ))
            } else {
                Err(refused(format!(
                    "{inventory:?} denominates in {currency:?} and the price is {:?}; Tes carries \
                     a bare amount, so posting this one would state a denomination nobody \
                     chose",
                    money.currency()
                )))
            }
        }
        CurrencyRule::SellerScoped | CurrencyRule::Unmeasured => Err(refused(format!(
            "{inventory:?} has no fixed currency, so there is no denomination to post a price in"
        ))),
    }
}

/// The mirror of [`paid_price_token`] and of the free branch beside it: a
/// bare Creative Commons token, or `TES-PAID` carrying its amount.
///
/// A paid token whose amount is missing, unparseable or non-positive is
/// refused rather than read as free. The licence the API refuses without a
/// price must never reach it without one, and a listing quietly demoted to
/// free is the one failure that costs the seller money.
fn pricing_from_token(token: &str) -> Result<TesPricing, AdapterError> {
    if let Some((licence, amount)) = token.split_once(PRICE_TOKEN_SEPARATOR) {
        if licence != TesLicence::TesPaid.as_str() {
            return Err(refused(format!(
                "only {} carries an amount, and {licence:?} is not it",
                TesLicence::TesPaid.as_str()
            )));
        }
        let minor_units: i64 = amount.parse().map_err(|error| {
            refused(format!(
                "the paid licence names {amount:?} as its minor units: {error}"
            ))
        })?;
        return TesPrice::new(minor_units)
            .map(TesPricing::Paid)
            .map_err(|error| refused(error.to_string()));
    }
    match token {
        "CC-BY" => Ok(TesPricing::Free(FreeLicence::CcBy)),
        "CC-BY-SA" => Ok(TesPricing::Free(FreeLicence::CcBySa)),
        "CC-BY-ND" => Ok(TesPricing::Free(FreeLicence::CcByNd)),
        other => Err(refused(format!(
            "unsupported licence token {other:?}; a paid listing carries \
             {}{PRICE_TOKEN_SEPARATOR}<minor units>",
            TesLicence::TesPaid.as_str()
        ))),
    }
}

/// A projection this adapter will not send. The seam's one rejection code for
/// a payload Tes cannot accept, so a refusal always names its own cause in
/// the detail rather than in a code the driver would have to interpret.
fn refused(detail: String) -> AdapterError {
    AdapterError::Rejected {
        code: FailureCode::UploadRejected,
        detail: FailureDetail(detail),
    }
}

/// A rights grant this adapter will not choose. Written through `refused` for
/// the same reason every other refusal is: the seam has one rejection code
/// and the cause belongs in the detail rather than in a code the driver would
/// have to interpret.
fn unprojectable_licence(detail: String) -> AdapterError {
    refused(detail)
}

/// One axis's elected value as the target's own native id, which is the form
/// `NativeAxis` carries and the form the wire takes. A value with no native
/// id is not a token, so it is absent rather than approximated from a label.
fn native_of(natives: &[NativeAxis], axis: TermKind) -> Option<&str> {
    natives
        .iter()
        .find(|native| native.axis == axis)?
        .value
        .native_id
        .as_deref()
}

fn licence_from(natives: &[NativeAxis]) -> Option<&str> {
    native_of(natives, TermKind::Licence)
}

/// The `Grades` entry, carrying the age pair only where the declaration
/// derived a span. Its absence is what the registry's `required: false` means,
/// and it is what stops a 16+-only listing stating age zero.
fn grades_entry(channel: &TesAges, ages: &[i64], main_age: Option<i64>) -> String {
    let mut entry = serde_json::Map::new();
    entry.insert(channel.field().to_owned(), serde_json::json!(channel.ids()));
    if let Some(main_age) = main_age {
        entry.insert("ages".to_owned(), serde_json::json!(ages));
        entry.insert("mainAge".to_owned(), serde_json::json!(main_age));
    }
    Value::Object(entry).to_string()
}

/// The `Taxonomy` entry, carrying `mainType` only where the projection
/// resolved one. The key's absence is what the registry's `required: false`
/// means, and it is what stops a placeholder naming a real Tes type.
fn taxonomy_entry(categories: &[i64], main_type: Option<i64>) -> String {
    let mut entry = serde_json::Map::new();
    entry.insert("categories".to_owned(), serde_json::json!(categories));
    if let Some(main_type) = main_type {
        entry.insert("mainType".to_owned(), serde_json::json!(main_type));
    }
    Value::Object(entry).to_string()
}

/// A projection Tes cannot express. The category ids it addresses are
/// numbers, so a term the crosswalk left without one is refused before an
/// attempt is opened rather than sent as something else.
fn unprojectable_category(detail: String) -> AdapterError {
    refused(detail)
}

/// The same, for the age channel. The two are separate names because the two
/// refusals name different fields to the seller.
fn unprojectable_grade(detail: String) -> AdapterError {
    refused(detail)
}

/// A resource with no bundle behind it. Distinct from a failed read: the
/// answer is known, and it is that only a published resource has files to
/// download.
fn no_published_bundle(id: DraftId) -> AdapterError {
    AdapterError::Rejected {
        code: FailureCode::PreconditionElementAbsent,
        detail: FailureDetail(format!(
            "resource {} has no published bundle; only a published resource can be downloaded",
            id.0
        )),
    }
}
