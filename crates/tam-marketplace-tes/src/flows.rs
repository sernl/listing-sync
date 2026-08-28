//! The adapter flows over the M0-confirmed endpoints: create-and-populate,
//! publish, delete with positive verification, and the fetch-reason-gated
//! read. Every step classifies its own response; no step trusts a status
//! another step produced.
//!
//! The `FieldSet` contract is this crate's on both sides: `project_fields`
//! renders it from a projected listing and the submit parses it back.
//! `Title` is plain text, `Description` is markdown, `Price` is a free-tier
//! licence token (`CC-BY`, `CC-BY-SA`, `CC-BY-ND`), `Taxonomy` is JSON
//! `{"categories": [..], "mainType": n}`, and `Grades` is JSON
//! `{"ageRanges": [..], "ages": [..], "mainAge": n}`.

use serde_json::Value;
use sha2::{Digest, Sha256};
use tam_marketplace::transport::Transport;
use tam_marketplace::{
    AdapterError, AmbiguityCause, FetchReason, FieldSet, FileContent, FileSource, FirstPartyExport,
    FormId, FormSchemaFingerprint, IdempotencyKey, ImportedListing, ListingLocator,
    MarketplaceAdapter, ObservedListing, ProjectedListing, RemoteLifecycle, RemoteListingId,
    SubmitEvidence,
};
use tam_types::{
    ContentHash, FailureCode, FailureDetail, FieldKey, InventoryId, OrgId, PriceIntent, Timestamp,
};

use crate::classify::{
    classify_create, classify_read, classify_read_bytes, classify_transport, classify_write,
    classify_write_json, classify_write_status,
};
use crate::endpoints::{self, CatalogueEntry, DraftId, PresignedUpload, TesLicence, TesListing};
use crate::schema;

/// The two Tes inventories share this one adapter type; everything that
/// differs between GB and US is data, per the design's crate table.
pub struct TesAdapter<T, F> {
    inventory: InventoryId,
    transport: T,
    file_source: F,
}

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

    /// Publishes the draft, then proves it by reading the state back. The
    /// publish POST's own 2xx is never the verdict.
    pub async fn publish(&self, id: DraftId) -> Result<Value, AdapterError> {
        let published = self.send(endpoints::publish_request(id)).await?;
        classify_write_status(&published)?;
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

    /// Deletes a never-published draft and reports success only after the
    /// `/draft` read returns 404. The authoritative `DELETE /resources/{id}`
    /// 404s for a draft-only resource without removing it, and a resource read
    /// 404s for a draft whether or not it was deleted, so verifying deletion
    /// against the resource route is a false "gone" — the deletion and its
    /// proof both run on the `/draft` route the draft actually lives at.
    pub async fn delete(&self, id: DraftId) -> Result<(), AdapterError> {
        self.send(endpoints::delete_draft_request(id)).await?;
        let after = self.send(endpoints::read_draft_request(id)).await?;
        match after.status {
            404 => Ok(()),
            200 => Err(AdapterError::Rejected {
                code: FailureCode::VerificationMismatch,
                detail: FailureDetail("the draft is still readable after delete".to_owned()),
            }),
            _ => Err(AdapterError::Ambiguous(
                AmbiguityCause::ReadBackIndeterminate,
            )),
        }
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
            // Marker search is the reconciliation flow's tool and lands with
            // the job engine (M1d).
            ListingLocator::Marker { .. } => Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail("marker reconciliation is not implemented yet".to_owned()),
            }),
        }
    }

    fn listing_from_field_set(fields: &FieldSet) -> Result<TesListing, AdapterError> {
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
        let licence = match entry(FieldKey::Price)? {
            "CC-BY" => TesLicence::CcBy,
            "CC-BY-SA" => TesLicence::CcBySa,
            "CC-BY-ND" => TesLicence::CcByNd,
            other => {
                return Err(AdapterError::Rejected {
                    code: FailureCode::UploadRejected,
                    detail: FailureDetail(format!(
                        "unsupported licence token {other:?}; paid listings land with price wiring"
                    )),
                })
            }
        };
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
            description_markdown: entry(FieldKey::Description)?.to_owned(),
            category_ids: ids(&taxonomy, "categories"),
            age_range_ids: ids(&grades, "ageRanges"),
            ages: ids(&grades, "ages"),
            main_type: taxonomy
                .get("mainType")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            main_age: grades.get("mainAge").and_then(Value::as_i64).unwrap_or(0),
            licence,
        })
    }
}

impl<T: Transport, F: FileSource> MarketplaceAdapter for TesAdapter<T, F> {
    fn inventory(&self) -> InventoryId {
        self.inventory
    }

    /// Tes's own wire shape, rendered here rather than in the engine that
    /// seeds the item: a licence token in `Price`, the numeric category ids
    /// in `Taxonomy`, and the age ranges with the derived age list in
    /// `Grades`. The submit's own `listing_from_field_set` parses exactly
    /// this back, so both halves of the contract sit in one file.
    ///
    /// A category the crosswalk left without a numeric id is refused: Tes
    /// addresses categories by number and there is nothing to send.
    fn project_fields(&self, listing: &ProjectedListing) -> Result<FieldSet, AdapterError> {
        let licence = match listing.price {
            PriceIntent::Free => "CC-BY".to_owned(),
            // The Tes write path's licence vocabulary is Creative Commons
            // only so far; a paid seed reaches this adapter's own closed
            // refusal and settles honestly rather than being silently freed.
            PriceIntent::Paid(_) => "TES-PAID".to_owned(),
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
        let mut age_ranges: Vec<i64> = Vec::new();
        for term in &listing.grades {
            if let Some(native) = term.native_id.as_deref() {
                if let Ok(id) = native.parse() {
                    age_ranges.push(id);
                }
            }
        }
        let (ages, main_age): (Vec<i64>, i64) = match listing.ages {
            Some(span) => (
                (i64::from(span.low_years)..=i64::from(span.high_years)).collect(),
                i64::from(span.low_years),
            ),
            None => (Vec::new(), 0),
        };
        Ok(FieldSet {
            entries: vec![
                (FieldKey::Title, listing.title.clone()),
                (FieldKey::Description, listing.body.clone()),
                (FieldKey::Price, licence),
                (
                    FieldKey::Taxonomy,
                    serde_json::json!({ "categories": categories, "mainType": 0 }).to_string(),
                ),
                (
                    FieldKey::Grades,
                    serde_json::json!({
                        "ageRanges": age_ranges,
                        "ages": ages,
                        "mainAge": main_age
                    })
                    .to_string(),
                ),
            ],
            files: listing.files.clone(),
        })
    }

    /// The pre-flight is the M0 loop as a probe: create a draft, write every
    /// field this adapter writes, read the result back, fingerprint it, and
    /// delete the probe. The write comes before the assertion because the
    /// live API omits null scalar keys from a draft's JSON, so an empty
    /// draft cannot witness the written-field set.
    async fn assert_form_schema(
        &self,
        _org: OrgId,
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
        let deleted = self.delete(id).await;
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
        _org: OrgId,
        _key: IdempotencyKey,
        fields: FieldSet,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        let listing = Self::listing_from_field_set(&fields)?;
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
            landed_on_route: Some(format!("{}/api/v2/resources/{}", endpoints::ORIGIN, id.0)),
            // The durable identifier the create returned, so the pre-settle
            // verification read addresses the resource directly rather than
            // search for a marker the JSON API never carried.
            landed: Some(RemoteListingId::Tes {
                url: format!("{}/api/v2/resources/{}", endpoints::ORIGIN, id.0),
            }),
            observed_lag: false,
        })
    }

    async fn read_back(
        &self,
        _org: OrgId,
        locator: ListingLocator,
        _reason: FetchReason,
        observed_at: Timestamp,
    ) -> Result<ObservedListing, AdapterError> {
        let id = Self::draft_id_from_locator(&locator)?;
        let state = self.resource_state(id).await?;
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
        let id = match locator {
            ListingLocator::Durable(durable) => durable,
            ListingLocator::Marker { .. } => RemoteListingId::Tes {
                url: format!("{}/api/v2/resources/{}", endpoints::ORIGIN, id.0),
            },
        };
        Ok(ObservedListing {
            id,
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
        if !matches!(reason, FetchReason::FirstPartyExport { .. }) {
            return Err(AdapterError::Rejected {
                code: FailureCode::Other,
                detail: FailureDetail(
                    "a catalogue read is justified only by the first-party-export capability"
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
    /// the bundle is fetched, the upstream's 302 to the signed CDN url being
    /// followed by the transport, so what arrives here is the ZIP itself.
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
        // resource is unpublished, not that the read failed.
        let body = match classify_read(&manifest) {
            Ok(body) => body,
            Err(error) => {
                return Err(if matches!(error, AdapterError::Rejected { .. }) {
                    no_published_bundle(id)
                } else {
                    error
                })
            }
        };
        let path = endpoints::parse_download_manifest(&body, id).map_err(|error| match error {
            endpoints::DownloadManifestError::NoPublishedBundle => no_published_bundle(id),
            endpoints::DownloadManifestError::OffOrigin(_) => AdapterError::Rejected {
                code: FailureCode::UnexpectedOrigin,
                detail: FailureDetail(error.to_string()),
            },
        })?;
        let bundle = self.send(endpoints::download_bundle_request(&path)).await?;
        classify_read_bytes(&bundle).map(<[u8]>::to_vec)
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
        let category_native_ids = state
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
        Ok(ImportedListing {
            remote: RemoteListingId::Tes {
                url: format!("https://www.tes.com/teaching-resource/-{}", id.0),
            },
            title,
            body,
            licence: state
                .get("licence")
                .and_then(Value::as_str)
                .map(str::to_owned),
            price: state.get("price").and_then(Value::as_f64),
            category_native_ids,
            age_range_native_ids: string_list(state.get("ageRanges")),
            year_groups: string_list(state.get("yearGroups")),
            curriculum: string_list(state.get("curriculum")),
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

/// A projection Tes cannot express. The category ids it addresses are
/// numbers, so a term the crosswalk left without one is refused before an
/// attempt is opened rather than sent as something else.
fn unprojectable_category(detail: String) -> AdapterError {
    AdapterError::Rejected {
        code: FailureCode::UploadRejected,
        detail: FailureDetail(detail),
    }
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
