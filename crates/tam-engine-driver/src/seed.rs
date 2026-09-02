//! The device half of seeding: rendering the field set and fingerprinting it.
//!
//! Both functions touch no storage and reach only `adapter.project_fields`,
//! which is why they belong here rather than beside `prepare_item`. The custody
//! line runs between them: the server decides what the item is and answers an
//! [`ItemPreparation`]; the device renders that into the marketplace's own
//! field set and takes the intent hash over what it rendered, so the recorded
//! intent is the bytes the submit will carry rather than a shape the server
//! guessed at. A server that rendered the field set would be composing, which
//! is the architecture at S3 rather than S1.
//!
//! Neither reads the policy from the inventory any more. The form, the create
//! strategy, the step budget and the verify policy all arrive in the
//! preparation, because they are the server's decisions and a device that
//! recomputed them would be setting its own budget.

use tam_marketplace::{FieldSet, MarketplaceAdapter, ProjectedListing};
use tam_types::ContentHash;

use crate::driver::{intent_as_json, EngineError, MachineSeed};
use crate::vocabulary::ItemPreparation;

/// The adapter half: it renders the field set, and the intent hash is taken
/// over what it rendered.
///
/// # Errors
///
/// Returns the adapter's refusal when the projection names something this
/// marketplace cannot express, which is a rejection before an attempt opens
/// rather than an approximation posted to a seller's account.
pub fn seed_from_projection<A: MarketplaceAdapter>(
    adapter: &A,
    preparation: &ItemPreparation,
    listing: &ProjectedListing,
) -> Result<MachineSeed, EngineError> {
    let fields = adapter.project_fields(listing)?;
    let intent_hash = content_hash(
        serde_json::json!({
            "entries": fields
                .entries
                .iter()
                .map(|(key, value)| (format!("{key:?}"), value))
                .collect::<Vec<_>>(),
            "files": fields
                .files
                .iter()
                .map(|file| file.0.to_hyphenated())
                .collect::<Vec<_>>(),
        })
        .to_string()
        .as_bytes(),
    );
    Ok(MachineSeed {
        form: preparation.form,
        fields,
        intent_hash,
        strategy: preparation.strategy,
        budget: preparation.budget,
        verify: preparation.verify,
    })
}

/// The removal's seed. A sibling of [`seed_from_projection`] rather than an
/// `Option` argument on it, because a removal has nothing for an adapter to
/// render: `project_fields` renders a listing, and a removal describes none.
/// The intent hash is taken over the removal's own recorded intent, so what
/// the ledger fingerprints is what the ledger stores.
#[must_use]
pub fn seed_for_removal(preparation: &ItemPreparation) -> MachineSeed {
    let fields = FieldSet {
        entries: vec![],
        files: vec![],
        // A removal renders no listing, so there are no body bytes for a
        // format to describe.
        body_format: None,
    };
    let intent_hash = content_hash(
        intent_as_json(&preparation.operation, &fields)
            .to_string()
            .as_bytes(),
    );
    MachineSeed {
        form: preparation.form,
        fields,
        intent_hash,
        strategy: preparation.strategy,
        budget: preparation.budget,
        verify: preparation.verify,
    }
}

/// The intent hash, blake3 over the rendered bytes.
///
/// The same digest `tam-pipeline` takes, computed here directly: reaching it
/// through the pipeline would drag an image decoder and a zip implementation
/// into every client binary for one call.
fn content_hash(bytes: &[u8]) -> ContentHash {
    ContentHash(*blake3::hash(bytes).as_bytes())
}
