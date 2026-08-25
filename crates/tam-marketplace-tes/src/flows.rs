//! The adapter flows over the M0-confirmed endpoints: create-and-populate,
//! publish, delete with positive verification, and the fetch-reason-gated
//! read. Every step classifies its own response; no step trusts a status
//! another step produced.
//!
//! The `FieldSet` contract this adapter accepts is interim until the
//! taxonomy hub (M1g) and the projection own it: `Title` is plain text,
//! `Description` is markdown, `Price` is a free-tier licence token
//! (`CC-BY`, `CC-BY-SA`, `CC-BY-ND`), `Taxonomy` is JSON
//! `{"categories": [..], "mainType": n}`, and `Grades` is JSON
//! `{"ageRanges": [..], "ages": [..], "mainAge": n}`.

use serde_json::Value;
use sha2::{Digest, Sha256};
use tam_marketplace::transport::Transport;
use tam_marketplace::{
    AdapterError, AmbiguityCause, FetchReason, FieldSet, FileContent, FileSource, FormId,
    FormSchemaFingerprint, IdempotencyKey, ListingLocator, MarketplaceAdapter, ObservedListing,
    RemoteLifecycle, RemoteListingId, SubmitEvidence,
};
use tam_types::{ContentHash, FailureCode, FailureDetail, FieldKey, InventoryId, OrgId, Timestamp};

use crate::classify::{
    classify_create, classify_read, classify_transport, classify_write, classify_write_json,
    classify_write_status,
};
use crate::endpoints::{self, DraftId, PresignedUpload, TesLicence, TesListing};
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

    async fn upload_file(
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

    /// Deletes the resource and reports success only after the API read
    /// returns 404. `DELETE .../{id}/draft` answers 204 while removing only
    /// the draft overlay, and the public URL soft-404s, so neither is ever
    /// consulted — the misleading-204 rule as code.
    pub async fn delete(&self, id: DraftId) -> Result<(), AdapterError> {
        self.send(endpoints::delete_resource_request(id)).await?;
        let after = self.send(endpoints::read_resource_request(id)).await?;
        match after.status {
            404 => Ok(()),
            200 => Err(AdapterError::Rejected {
                code: FailureCode::VerificationMismatch,
                detail: FailureDetail("the resource is still readable after delete".to_owned()),
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
                .rsplit('-')
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

    /// The pre-flight is the M0 loop as a probe: create an empty draft, read
    /// its field-name set, fingerprint it, and delete the probe — asserting
    /// on the way that every field this adapter writes still exists.
    async fn assert_form_schema(
        &self,
        _org: OrgId,
        form: FormId,
    ) -> Result<FormSchemaFingerprint, AdapterError> {
        let created = self.send(endpoints::create_draft_request()).await?;
        let id = classify_create(&created)?;
        let state = self.resource_state(id).await;
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
    /// call site.
    async fn submit(
        &self,
        _org: OrgId,
        _key: IdempotencyKey,
        fields: FieldSet,
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
            // The durable identifier rides here: it is what the driver
            // observed, and the settle step derives the receipt from it.
            landed_on_route: Some(format!("{}/api/v2/resources/{}", endpoints::ORIGIN, id.0)),
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
