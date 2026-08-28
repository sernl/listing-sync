//! The TPT read adapter: the seller's own catalogue off `/graph/graphql` and
//! the per-resource analytics off `/gateway/graphql`, both gated on the
//! first-party-export capability because an enumeration-shaped read is
//! exactly that tier and nothing else justifies one.
//!
//! The write path is absent by design, not by omission: the M7 plan holds it
//! capture-gated behind a recording of one product created and published, so
//! this crate implements no `MarketplaceAdapter` and the two `FirstPartyExport`
//! methods it cannot serve say so in the type rather than approximating.

use serde_json::Value;
use tam_marketplace::transport::{HttpRequest, HttpResponse, Transport};
use tam_marketplace::{AdapterError, FetchReason, FirstPartyExport, ImportedListing};
use tam_types::{FailureCode, FailureDetail, InventoryId};

use crate::classify::{classify_graphql_read, classify_transport};
use crate::endpoints::{
    self, AllTimeMetric, ResolvedStatsQuery, CATALOGUE_PAGE_LIMIT, STATS_BATCH_MAX,
};
use crate::read_model::{self, ProductId, ResourceStat, TptCatalogueEntry};

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
pub struct TptAdapter<T> {
    transport: T,
    page_limit: u32,
}

fn not_first_party(what: &str) -> AdapterError {
    AdapterError::Rejected {
        code: FailureCode::Other,
        detail: FailureDetail(format!(
            "a {what} is justified only by the first-party-export capability"
        )),
    }
}

impl<T: Transport> TptAdapter<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self {
            transport,
            page_limit: CATALOGUE_PAGE_LIMIT,
        }
    }

    /// The TPT client hard-codes a hundred rows per page and this adapter
    /// follows it; the page size is a constructor argument so a recorded walk
    /// can be replayed over a handful of rows rather than a hundred.
    #[must_use]
    pub const fn with_page_limit(transport: T, page_limit: u32) -> Self {
        Self {
            transport,
            page_limit,
        }
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
}

impl<T: Transport> FirstPartyExport for TptAdapter<T> {
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
