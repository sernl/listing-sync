//! The structural probe: assert the form schema against the founder's own
//! account on a schedule, so a drifted form is a recorded observation before
//! it is a customer's failed publish. Drift raises the fleet halt for the
//! inventory — create disabled while everything else keeps running is the
//! Mozilla-modelled gate the design names.

use tam_marketplace::{AdapterError, FormId, MarketplaceAdapter};
use tam_storage::{HaltCause, HaltRepo, StorageError};
use tam_types::{OrgId, Timestamp};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeVerdict {
    Clean,
    Drifted { detail: String },
    Failed { detail: String },
}

pub async fn probe(
    adapter: &impl MarketplaceAdapter,
    halts: &HaltRepo,
    org: OrgId,
    form: FormId,
    now: Timestamp,
) -> Result<ProbeVerdict, StorageError> {
    match adapter.assert_form_schema(org, form).await {
        Ok(_fingerprint) => Ok(ProbeVerdict::Clean),
        Err(AdapterError::SchemaDrift(drift)) => {
            let detail = format!("added {:?}, removed {:?}", drift.added, drift.removed);
            halts
                .raise_fleet_inventory(
                    adapter.inventory(),
                    &HaltCause {
                        raised_by: "canary".to_owned(),
                        reason: detail.clone(),
                        at: now,
                    },
                )
                .await?;
            Ok(ProbeVerdict::Drifted { detail })
        }
        Err(error) => Ok(ProbeVerdict::Failed {
            detail: format!("{error:?}"),
        }),
    }
}
